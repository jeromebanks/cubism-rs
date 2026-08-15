---
name: timeseries-slice
description: Run one bounded "session slice" of work on cubism's time-series/Iceberg branch (feature/timeseries-phase-0a) — read the latest handoff, pick and implement the next small bounded test requirement or deferred item, file GitHub issues for anything newly deferred, get an advisor review, commit, push, and write the next numbered handoff doc, updating docs/handoff_latest.md to point at it. Use this whenever the user says "do a session slice," "continue the timeseries work," "pick up where the last handoff left off," or gives the same multi-part instruction this skill was created to replace (read handoff / continue / file issues / advisor review / commit and push / write handoff). Do not use for open-ended feature work with its own explicit scope, or for work outside crates/cubism-iceberg and its docs — this skill is specifically the repeatable slice-and-handoff loop established across docs/TIMESERIES_PHASE_0A through PHASE_6_HANDOFF.md.
---

# Timeseries session slice

Encodes the exact workflow used for every `TIMESERIES_PHASE_N_HANDOFF.md`
session on `feature/timeseries-phase-0a` so it doesn't need to be
re-specified by hand each time. Read this whole file before starting — the
ordering matters (advisor-before-code, verify-before-cite, commit-before-final-advisor-call).

## 0. Orient

```bash
cd /Users/jeromebanks/dev/cubism_saas/cubism
readlink docs/handoff_latest.md     # resolves the symlink target, e.g. TIMESERIES_PHASE_6_HANDOFF.md
git status                          # confirm clean tree; stash/investigate anything unexpected
git log --oneline -10
git rev-list --left-right --count origin/feature/timeseries-phase-0a...HEAD   # confirm in sync before starting
gh issue list --repo jeromebanks/cubism-rs --state open --limit 30
```

Then use the `Read` tool on `docs/<the resolved target>` to actually read the
latest handoff. Read its **"Deferred / not done this session"** section —
that list is where the next slice's candidates live. Cross-check each
deferred item against the open-issues list: some will already be filed
(don't re-file), some won't be (file them this session — step 3).

**Naming note:** these handoff filenames are *session-slice* numbers, not
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` phase numbers — a session named
`PHASE_6_HANDOFF.md` may still be doing prep work for plan-Phase 4. Every
handoff in this series opens with a parenthetical disclaiming this; carry
it forward in the one you write (copy the wording from
`docs/handoff_latest.md`'s own opening section and adjust the doc names it
points to).

## 1. Call advisor before picking the slice

**First check `docs/TIMESERIES_ROADMAP.md` if it exists.** For any phase the
roadmap covers, find the first milestone not yet marked done and confirm its
dependencies are done — that's the candidate slice, not a fresh scan of the
deferred list. As of this writing the roadmap covers Phase 4 and part of
Phase 5 (its own "Phase 5 Milestones" section notes what Phase 5 completion
criterion — and what beyond Phase 5 — it doesn't reach yet). Only fall back
to the ad hoc deferred-list/issue scan below for phases/criteria the
roadmap doesn't reach, or once every milestone in it is done.

Do not choose the code slice yourself — whether from the roadmap or the
deferred list — without a second opinion: in the session that produced this
skill, the advisor caught that one candidate issue (#10) already covered
scope a naively-filed new issue would have duplicated, and it picked a
better-scoped code slice than the first candidate considered. Orientation
(step 0) is not substantive work, so do it first, *then* call `advisor()`
and ask it to:

- if a roadmap milestone was picked: confirm its dependencies are actually
  done (not just listed as done) and that the milestone is still sized to
  one bounded slice — the roadmap's sizing is a plan, not a guarantee;
- confirm which deferred items already have GitHub issues (read the full
  body of any issue whose title looks like it might already cover the
  scope — GitHub list views truncate titles, so a short grep of the list
  is not enough to rule out overlap);
- pick one bounded next code slice — a single test requirement or a single
  small deferred item, not a whole implementation-plan phase. Every slice
  so far has been sized to "one new integration test plus its supporting
  code," not a multi-file feature;
- flag anything about the target file's locking/concurrency/scale model
  that would make the obvious test assertion wrong (e.g. the disjoint-window
  slice: `SqliteStore`'s `max_connections(1)` + `BEGIN IMMEDIATE` means
  "concurrent" cannot mean "parallel at the DB level" — the advisor caught
  this before a test got written that would have asserted something false).

## 2. Implement the slice

- Keep it to the bounded scope the advisor confirmed. Match the existing
  test/doc-comment style in the target file exactly — this codebase's
  convention is: every new test's doc comment states precisely what it
  proves *and* what it does not prove, especially about timing/concurrency
  claims that are easy to overstate. If a barrier-based test's barrier does
  no actual discriminating work (i.e., it would pass identically without
  the barrier), say so in the comment — don't let a test's name imply more
  rigor than it has.
- **Verify every line-number citation before writing it into code comments
  or docs.** Don't approximate with `~`. This repo has a standing plain
  `grep` problem: the `rtk` hook mangles `grep`'s output in this
  environment (returns something like `"N matches in 0 files"` instead of
  the actual lines). Use `rtk proxy grep -n "..." <file>` to get real
  output, or the `Read` tool with an offset, and confirm the exact number
  before citing it anywhere.

## 3. File GitHub issues for newly (or still) untracked deferred items

Only file for items the advisor step confirmed aren't already covered.
Follow the existing issue style in this repo (see any of #7–#14 for
tone/structure): `## Summary`, `## Findings` or `## Scope`, `## Suggested
next steps`, explicit non-goals cross-linking whichever issue already
covers adjacent territory, `## Environment` naming the branch/commit/doc
this was found in.

```bash
gh issue create --repo jeromebanks/cubism-rs --title "..." --body-file /path/to/body.md
```

## 4. Run the full verification battery

Same battery every slice has used — run all of it, not a subset:

```bash
cargo test -p cubism-iceberg                                              # full count — cite this number in the handoff, don't guess it from a --lib-only run
cargo test -p cubism-iceberg --test concurrency
cargo test -p cubism-iceberg --test durability
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings
cargo build -p cubism-cli
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings
cargo test --workspace --exclude cubism-py
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings
```

All must be clean before moving on. If `cargo test -p cubism-iceberg`'s
total doesn't match unit+integration counts you'd expect, run it without
piping through `tail` first — a truncated capture has produced a wrong
number in a handoff before.

## 5. Determine the next handoff number and write it

Derive `N` from the symlink read in step 0 (`readlink docs/handoff_latest.md`),
not by globbing/`grep`-ing the directory — `TIMESERIES_PHASE_0A_HANDOFF.md`
and `_0B_` both reduce to `0` under a naive numeric extraction, and this
environment's `grep` hook has repeatedly mangled plain `grep` output in this
series (see step 2's citation-verification note). The symlink is
already the source of truth for "current," so reuse it:

```bash
readlink docs/handoff_latest.md     # e.g. TIMESERIES_PHASE_6_HANDOFF.md -> next is _7_
```

New doc is `TIMESERIES_PHASE_<N+1>_HANDOFF.md`. Follow the section
structure every prior handoff in this series uses (copy the skeleton from
`docs/handoff_latest.md`, then rewrite each section for this slice):

1. Title, Date, Branch, **Status** paragraph (what actually landed, one
   sentence on what didn't), filename disclaimer, `**Superseded by:**` —
   leave blank for now, filled in step 8.
2. `## What this session built`
3. `## What was actually verified` — this is the section this series
   treats as load-bearing. State plainly what a new test does and does not
   prove; don't let a passing test read as more coverage than it has.
4. `## GitHub issues touched`
5. `## Deferred / not done this session` — numbered list; this is the
   input to the *next* slice's step 0/1, so make items concrete and
   actionable, not vague.
6. `## Worktree state` — list new/modified/untouched files; don't hardcode
   this session's own commit hash (self-referential-hash problem this
   series has already had to fix twice — `c2c92b3`, `d7efc04`). Note
   `.serena/` and `examples/web_analytics_demo/events.csv` stay
   deliberately uncommitted.
7. `## Tests (N in cubism-iceberg: ...)` — use the verified count from
   step 4.
8. `## Verification performed` — the exact battery from step 4, as a code
   block.
9. `## Primary files` — link the changed files, the plan doc section,
   `docs/handoff_latest.md`'s prior target, and any new/relevant issues.

Then go back and add the `**Superseded by:**` line to the *previous*
handoff (the one `docs/handoff_latest.md` currently points to) — one
sentence pointing at the new doc and which of its own deferred items got
picked up or filed. Leave the rest of that previous doc's historical claims
untouched; this series' convention is additive pointers, never rewriting
history in place.

## 6. Update the symlink

```bash
cd docs
ln -sf TIMESERIES_PHASE_<N+1>_HANDOFF.md handoff_latest.md
```

## 7. Commit

```bash
git add <changed source/test files> docs/TIMESERIES_PHASE_<N+1>_HANDOFF.md docs/TIMESERIES_PHASE_<N>_HANDOFF.md docs/handoff_latest.md
git status   # confirm nothing unexpected staged — never `git add -A`; .serena/ and the events.csv demo output must stay untracked
git commit -m "..."
```

## 8. Advisor review, then push

Call `advisor()` again now that there's a real diff to review. This
series' pattern (see `c2c92b3`, `d7efc04`, and the commit this skill's own
originating session made) is: apply any correction the advisor finds as a
**separate follow-up commit**, not a rewrite of the first one — that's
normal here, not a failure to fix in place. Common things the advisor has
caught in this series: unverified `~`-approximated citations, a handoff
with no forward-pointer from the previous one, a missing filename
disclaimer, an undercounted test total, a test claim that overstates what
timing/concurrency was actually shown.

Once the advisor pass is clean (or its follow-up commit is in), push:

```bash
git push origin feature/timeseries-phase-0a
```

Push and commit are pre-authorized for this workflow — that's the standing
instruction this skill exists to satisfy — but everything in `git status`
should still be exactly what step 7 intended to stage before it's pushed.

## 9. Report back

Short summary: what slice landed, what got filed, what's still deferred,
link to the new handoff doc. Don't restate the whole handoff doc in chat —
it's already durable on disk and pushed.
