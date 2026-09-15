# SDLC process notes

> **Historical retrospective (retired process).** The shared time-series branch
> merged in PR #24. These observations informed the current [`SDLC.md`](../SDLC.md),
> but this document is not an active workflow. Future time-series work uses the
> same `work-slice` issue/PR process as every other subsystem.

Findings from running the `timeseries-slice` skill's session-slice loop
across 13+ tracked slices (`docs/TIMESERIES_PHASE_0A_HANDOFF.md` through
`TIMESERIES_PHASE_20_HANDOFF.md`, `git log --oneline | grep timeseries:`).
This is a retrospective on the *process*, not the feature — written because
the pattern held consistently enough across a real branch's history that it
looks worth generalizing beyond `crates/cubism-iceberg`, not because any
single incident was severe.

## The headline evidence

Every one of the 13 tracked slices — Milestones 1 through 10, plus the
rollback, retry-loop, and Phase-5-roadmap-extension slices — shows a `land
X` commit followed by an `apply advisor follow-up fixes for X` commit in
`git log`. Zero exceptions. The Milestone 10 slice needed two follow-up
rounds, the first time that's happened in the series.

Two categories of finding recur across sessions, not just once:

- **Citation drift.** Line-number citations in handoff docs or code
  comments were correct when written, then went stale after a later edit
  in the same session added or removed lines above them. Caught by advisor
  review in Phase 18, 19, and twice in Phase 20.
- **Formatting/toolchain drift (`#3`).** `cargo fmt -p <crate>`
  reformatted six untouched files in Phase 19. The documented workaround
  (switch to single-file `rustfmt --edition 2024 <path>`) did not fix the
  underlying problem: Phase 20 hit it twice more, including a confirmed
  case where formatting exactly one file — correctly scoped — still
  reflowed that file's own pre-existing, already-committed content,
  because the file was never `rustfmt --check`-clean under this toolchain
  to begin with.

One prior recurring problem — hardcoding a session's own commit hash into
its handoff doc, which is unknowable until after the commit exists — *did*
get a durable fix (`docs/TIMESERIES_PHASE_14_HANDOFF.md` era): the
convention changed to "never hardcode it, point at `git log` instead," and
it has not recurred since. That's the target shape for the two open
categories above: a structural fix, not a repeated workaround.

## What's actually working, worth generalizing as-is

1. **Two advisor gates, not one.** A second opinion *before* implementation
   (confirming scope, checking for issue-title-vs-body duplication, sizing
   the slice) and a second opinion *after* a real diff exists, before
   pushing. The pre-implementation gate has caught things a naive scan
   would have missed — e.g. an issue whose title didn't suggest it already
   covered the candidate scope, and at least one case where the "obvious"
   next milestone would have built on a premise (a divisibility guarantee,
   a dependency-direction assumption) that turned out false on inspection.
2. **Bounded slice sizing.** "One new test plus its supporting code," not
   a whole implementation-plan phase. This is a big part of why the
   100%-follow-up-rate finding above isn't actually alarming: follow-ups
   have consistently been small (a doc correction, a scope-fence
   adjustment), not a rewrite, because the original slice was small enough
   to review completely in one pass.
3. **Recording deviations in place, additively.** When a slice narrows
   scope from what a roadmap/plan doc originally described, the deviation
   gets written into that same doc's entry — what was skipped, why, and
   where the skipped work now lives — rather than silently shipping less
   than promised or rewriting the doc's history. Milestone 9's
   cross-resolution-decomposition cut and Milestone 10's value-
   materialization cut are both handled this way.
4. **Handoff docs with a "does not prove" section.** Every new test's
   claim is bounded explicitly: what it demonstrates *and* what it doesn't
   (timing/concurrency claims especially). This is the single practice
   most worth exporting to other feature areas verbatim — it's cheap to
   write and it's what makes a handoff trustworthy enough for the next
   session (or the next engineer) to build on without re-deriving state.
5. **A living roadmap doc as the source of truth for "what's next."**
   `docs/TIMESERIES_ROADMAP.md` gets milestone statuses flipped in place as
   they land, with landed line numbers and deviations recorded, rather than
   "next steps" living only in scattered handoff docs. New sessions read
   the roadmap first, not a chain of handoffs.

## Recommendations

### Fix `#3` at the root instead of re-discovering it per session

Three separate incidents across two sessions is a strong enough signal
that the workaround (narrow the `rustfmt` invocation) isn't actually
fixing anything — it's moving where the same problem surfaces. The cheap,
durable fix: add `cargo fmt --check --all` (or an equivalent scoped to
`--edition 2024`) as a CI gate, and run `cargo fmt --edition 2024` once
across the whole workspace to normalize it against whatever toolchain CI
uses. Once the tree is `--check`-clean everywhere, any future single-file
`rustfmt` call is safe by construction, and the mandatory `--check`-first
gate now in `.claude/skills/timeseries-slice/SKILL.md` step 2 becomes a
no-op safety net instead of a routinely-triggered one. Treat "a workaround
has been reapplied three times" as itself a signal worth acting on, not
just tracking — this generalizes beyond `#3`: if any tracked issue needs a
workaround more than twice, that's a trigger to schedule the real fix
rather than let it accumulate as recurring tax.

### Generalize the two-gate advisor pattern beyond this branch

The pre-implementation and pre-push advisor calls are cheap relative to
what they catch (a false premise, a duplicate issue, a scope creep). This
is a candidate house-wide pattern for any agent-driven feature work in
this repo, not something specific to `cubism-iceberg`: confirm scope and
dependencies before writing code; get a second read on the real diff
before it ships. The `timeseries-slice` skill already encodes this well
enough to lift into a generic, crate-agnostic version.

### Turn the handoff-doc template into a house-wide artifact

The "What was actually built / What was actually verified (with an
explicit does-not-prove section) / Deferred, numbered and concrete" shape
is not timeseries-specific — it's a general discipline for keeping
multi-session work legible without re-deriving state each time. Worth
extracting into a standalone template (`docs/templates/session-handoff.md`
or similar) that other feature branches can adopt without waiting for
their own bespoke skill to accrete the same lessons the hard way.

### Never let a document cite a value only knowable in the future

The commit-hash lesson (point at `git log`, don't hardcode) is a specific
case of a general rule: if a citation, count, or hash can only be known
after the artifact referencing it exists, don't inline it — reference it
indirectly (a symlink, a `git log` pointer, a "see CI" note) or compute it
mechanically at the last possible step. The new mechanical
citation-re-verification pass (skill step 8) is the same rule applied to
line numbers: verify against current state right before the point of no
return, not from memory or an earlier draft.

### Treat "run the full verification battery, fresh, every time" as non-negotiable

Two failure modes recurred before this rule was tightened: citing a
stale/carried-forward number, and citing a number produced by a command
piped through `tail` that silently truncated the real count. Both are
now explicit rules in the skill (re-run fresh, don't pipe through `tail`
for the headline count). This generalizes cleanly: any automated
verification step whose output feeds into a written claim should be run
un-truncated, immediately before the claim is written, not reused from
earlier in the session.

### Keep the GitHub issue hygiene rule explicit, not assumed

"Read the full body of any issue whose title might overlap, not just the
list view" has prevented at least one duplicate filing. GitHub's list
view truncates titles and gives no scope signal beyond a few words — this
is cheap insurance worth stating as a standing house rule for any agent
filing issues in this repo, not just this branch's skill.

## Suggested next steps

1. Add `cargo fmt --check` (workspace-wide, `--edition 2024`) to CI, and
   run a one-time normalization pass so the tree starts clean. This
   directly retires the recurring `#3` workaround tax.
2. Extract a crate-agnostic version of the two-gate advisor pattern and
   the handoff-doc template from `.claude/skills/timeseries-slice/SKILL.md`
   into a general-purpose skill or house style guide, so other feature
   areas don't have to reconstruct the same lessons independently.
3. Periodically (e.g., every 5-10 slices, or whenever a workaround gets
   reapplied) do a short process retro like this one — cheap, and it's
   what surfaced both the `#3` root-cause signal and the citation-drift
   pattern before they became larger.
4. **Decide on an issue-tracking system for the next-gen SDLC** — GitHub
   issues (current), Linear, Jira, or something else. Not analyzed here;
   flagged during this retro (2026-08-16) as an open question for a
   dedicated follow-up, not a decision to make inline. Whatever's chosen
   should account for what's already working well with GitHub issues in
   this series specifically (cheap `gh issue create`/`view` from an agent
   session, the Summary/Scope/Suggested-next-steps/non-goals/Environment
   template above, issue numbers as stable cross-references from commit
   messages and docs) rather than assuming a switch is free.

## Addendum: cross-model phase review (added 2026-08-16)

Prompted by comparing notes against `~/dev/postscript_interpreter`'s
`work-issue` skill, which gates merge on an independent Codex review —
deliberately blank-context, seeing only `gh pr diff`, never the
implementing session's narrative. `timeseries-slice` had no equivalent:
every review in this series has been `advisor()`, which inherits the
whole session transcript, including whatever justification was offered
for each choice along the way. That's a real gap, but `work-issue`'s
per-PR cadence doesn't map onto this branch (no PRs, direct push to a
shared feature branch) or its economics (slices are deliberately small
and cheap; a multi-minute cross-model pass on every one would erode that).

**Update (2026-09-04):** PR #24 merged the shared time-series branch. The
two-lane exception described in this retrospective is now retired. The useful
parts—session-sized slices, advisor feedback, deterministic checks, and
phase-level human visibility—were incorporated into the repo-wide process in
`SDLC.md`; its issue/PR state and merge gates now apply to time-series work too.

Landed as `.claude/skills/timeseries-slice/SKILL.md` step 8a instead:
triggered only when a slice closes a roadmap `## Phase N "done" condition`
section — Phase 4 took ~8 slices, Phase 5 is on track for ~5-6 — reviewing
the whole phase's accumulated diff via the same Codex plugin `work-issue`
uses (`--scope branch --base <phase start>`), with output written to
`docs/phase-reviews/TIMESERIES_PHASE_<N>_REVIEW.md` rather than a PR
comment. Forward-only: Phase 4 already closed without this gate and isn't
retroactively reviewed. First real run expected whenever Phase 5's "done"
condition is reached (Milestone 10b and/or `#8`).
