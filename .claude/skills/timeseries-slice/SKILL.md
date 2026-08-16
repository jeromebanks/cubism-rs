---
name: timeseries-slice
description: Run one bounded "session slice" of work on cubism's time-series/Iceberg branch (feature/timeseries-phase-0a) — read the latest handoff, pick and implement the next small bounded test requirement or deferred item, file GitHub issues for anything newly deferred, get an advisor review, commit, push, and write the next numbered handoff doc, updating docs/handoff_latest.md to point at it. Use this whenever the user says "do a session slice," "continue the timeseries work," "pick up where the last handoff left off," or gives the same multi-part instruction this skill was created to replace (read handoff / continue / file issues / advisor review / commit and push / write handoff). Do not use for open-ended feature work with its own explicit scope, or for work outside crates/cubism-iceberg and its docs — this skill is specifically the repeatable slice-and-handoff loop established across docs/TIMESERIES_PHASE_0A through PHASE_6_HANDOFF.md.
---

# Timeseries session slice

Encodes the exact workflow used for every `TIMESERIES_PHASE_N_HANDOFF.md`
session on `feature/timeseries-phase-0a` so it doesn't need to be
re-specified by hand each time. Read this whole file before starting — the
ordering matters (advisor-before-code, check-before-format,
verify-before-cite, re-verify-before-final-advisor-call,
commit-before-final-advisor-call). One occasional step is
condition-triggered rather than every-slice: step 8a's cross-model phase
review, which only runs when this slice closes out a roadmap `## Phase N
"done" condition` section — see step 1 for how that gets flagged and step
8a for the mechanics.

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
- **confirm whether landing this slice will satisfy (or narrow-and-close,
  the usual outcome in this series) a roadmap `## Phase N "done" condition`
  section.** If yes, step 8a's cross-model phase review applies this
  session — note that now so it isn't discovered as an afterthought after
  step 7's commit. If this slice is also the *first* one to add a brand
  new `## Phase N` section to the roadmap (i.e. it opens a phase, not
  closes one), record the commit immediately before that roadmap edit as
  that section's **Phase start (for step 8a):** line — this is what step
  8a's eventual review diffs from, and it's cheap to capture now versus
  git-archaeology later. (Phase 5's own start is already backfilled:
  `f0599b2`, the commit immediately before `4299d60` added the "Phase 5
  Milestones" section.)

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
- **Before running `rustfmt --edition 2024 <file>` on any file — including
  a file you're only adding to, not rewriting — run `rustfmt --edition
  2024 --check <file>` on it first, against a clean tree.** Plain `rustfmt
  <path>` with no `--edition` flag fails outright on this crate's
  let-chain syntax, so `--edition 2024` is required either way, but the
  `--check` pass is a separate, mandatory gate: if it reports *any* diff,
  the file already isn't clean under this toolchain, and a bare `rustfmt
  --edition 2024 <file>` will reflow lines you never touched, including
  content from prior sessions. This bit twice in the Milestone 10 slice
  alone — once as five files reformatted by an unidentified cause, once as
  a confirmed case where formatting exactly one file (scoped correctly,
  not `cargo fmt -p`) still reflowed that file's own pre-existing,
  already-committed test body. When `--check` reports pre-existing diffs,
  do not run plain `rustfmt` on that file: hand-write your new lines
  matching the surrounding style instead, and leave the rest of the file's
  formatting exactly as committed. `cargo clippy -D warnings` does not
  care about formatting, so this costs nothing at verification time.

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

**Before calling advisor, re-verify every line-number citation in the new
handoff doc and in any code/roadmap comments touched this session —
mechanically, not from memory.** Citations that were correct when first
written have drifted after a later edit in the same session more than
once in this series (Phase 18, 19, and twice in Phase 20's own follow-up
round). For each `file.rs:N` or `file.rs:N-M` citation added or changed
this session, re-run `rtk proxy grep -n "<anchor text>" <file>` (or `Read`
with an offset) against the file's *current* state and fix any that
shifted — most commonly because a citation was written before a later
edit added or removed lines above it. This is a mechanical pass you do
yourself; don't rely on the advisor call below to catch it, that's
strictly slower (a full extra round trip) than catching it here.

Call `advisor()` again now that there's a real diff to review. This
series' pattern (see `c2c92b3`, `d7efc04`, and the commit this skill's own
originating session made) is: apply any correction the advisor finds as a
**separate follow-up commit**, not a rewrite of the first one — that's
normal here, not a failure to fix in place. Common things the advisor has
caught in this series: unverified `~`-approximated citations, a handoff
with no forward-pointer from the previous one, a missing filename
disclaimer, an undercounted test total, a test claim that overstates what
timing/concurrency was actually shown.

Once the advisor pass is clean (or its follow-up commit is in): **if step
1 flagged this slice as phase-closing, do step 8a now, before the push
below** — its own fix commits (if any) should go out in the same push as
this slice's, not a second one. Otherwise skip straight to push.

```bash
git push origin feature/timeseries-phase-0a
```

Push and commit are pre-authorized for this workflow — that's the standing
instruction this skill exists to satisfy — but everything in `git status`
should still be exactly what step 7 (and step 8a, if it ran) intended to
stage before it's pushed.

## 8a. Cross-model phase review (only when this slice closes a phase)

**Skip this step entirely unless step 1 flagged that this slice satisfies
a roadmap `## Phase N "done" condition` section.** Most slices don't
reach this step — it's phase-sized, not slice-sized, deliberately, so it
doesn't add cost to the common case.

**Why this exists, and why it's not just another `advisor()` call:**
`advisor()` forwards this entire session's transcript — every
rationalization and framing choice made along the way. It's a genuinely
useful check, and every slice in this series has used it, but it can't
catch a mistake it's been talked into agreeing with, because it always
sees *why* a choice was made, not just the choice. A phase boundary is
sized right for a second kind of check: a reviewer that sees only the
accumulated diff, with none of these sessions' narrative behind it. This
mirrors `~/dev/postscript_interpreter/.claude/skills/work-issue/SKILL.md`
step 8's cross-model Codex gate, adapted from a PR-based merge checkpoint
(which this branch doesn't have — no PRs, direct push) to a phase-sized
diff instead.

**Forward-only.** This step did not exist when Phase 4 closed (Milestone
6, `docs/TIMESERIES_PHASE_14_HANDOFF.md` era) — Phase 4 is not
retroactively reviewed under this process. The first phase this applies
to is Phase 5, whenever its "done" condition is actually reached
(Milestone 10b and/or `#8` landing). Revisit whether a Phase 4 baseline
review is worth doing separately later; don't let this step's existence
imply one already happened.

Locate the Codex plugin runtime the same way `work-issue` does — don't
assume `$CLAUDE_PLUGIN_ROOT`:

```bash
CODEX_SCRIPT=$(find "$HOME/.claude/plugins" -path "*/codex/scripts/codex-companion.mjs" 2>/dev/null | head -1)
node "$CODEX_SCRIPT" status --json   # confirm it resolves before trusting the review call below
```

Read the phase's **Phase start:** commit from its roadmap section (step 1
backfills this when a phase opens). Run the review scoped to that whole
range, from inside the repo root, on a clean `HEAD` (this slice's own
commit(s) from step 7 already in):

```bash
rm -f /tmp/codex-phase-review-<N>.json && \
  git fetch origin feature/timeseries-phase-0a && \
  node "$CODEX_SCRIPT" review --wait --json --scope branch --base <phase-start-commit> \
  > /tmp/codex-phase-review-<N>.json
```

Same pitfalls apply as `work-issue`'s equivalent step (worth re-reading
that file's "Pitfalls" section once before the first time this runs in
this repo): expect several minutes, not a hang; the real content is
`.codex.stdout` (free-form markdown, `[P1]`/`[P2]`-tagged bullets as a
rough heuristic, not a schema); check `.codex.status == 0` and non-empty
`.codex.stdout` before trusting the output; a `status --all --json` job
can report `"running"` long after the process actually died — cross-check
`kill -0 <pid>` before waiting longer.

**If the Codex runtime is unavailable or the run fails:** fall back to a
blank-context `Agent` (not `advisor` — it must not inherit this session).
Self-contained prompt: nothing but "review `git diff
<phase-start-commit>..HEAD` in
`/Users/jeromebanks/dev/cubism_saas/cubism` for correctness, scope
overclaims, and consistency with `docs/TIMESERIES_ROADMAP.md`'s Phase N
'done' condition claims" — it reads the diff and the roadmap itself, not
anything from this session. Note the fallback explicitly in the output
file below.

**Output goes in a file, not a PR/issue comment — there's no PR on this
branch.** Write to `docs/phase-reviews/TIMESERIES_PHASE_<N>_REVIEW.md`
(create the `docs/phase-reviews/` subfolder the first time this runs),
containing: date, branch, phase number, the diff range (`<phase-start
commit> (one-line)..HEAD (one-line)`), which reviewer ran (Codex, or
"blank-context Agent fallback" with why), and the full review text
verbatim. **Write this file even if the review is clean with nothing to
fix** — a clean pass is still a durable record, same as `work-issue`'s
"post unconditionally" rule.

**Read the whole thing yourself; don't gate on `[P1]`-only.** Every
finding needs an explicit disposition recorded in the same file — fixed
(with the follow-up commit hash once it exists), not a bug (with why), or
deferred (with the issue it's tracked under, filing one via step 3's
convention if it isn't tracked yet). An unexplained skip isn't
acceptable, matching this series' "don't let a passing check read as more
coverage than it has" convention applied to review findings instead of
tests.

If fixes are needed: apply them as a separate follow-up commit (same
"never rewrite the landing commit" convention as every other advisor
follow-up in this series), re-run the step 4 battery, update the
disposition in the review file, and commit that update too. Then link the
review file from the roadmap doc's `## Phase N "done" condition` section
(an additive pointer, same convention as every other cross-link in this
doc) before continuing to push.

## 9. Report back

Short summary: what slice landed, what got filed, what's still deferred,
link to the new handoff doc. If step 8a ran, also link the new
`docs/phase-reviews/TIMESERIES_PHASE_<N>_REVIEW.md` and say in one line
whether it came back clean or needed a fix. Don't restate the whole
handoff doc (or the whole phase review) in chat — both are already
durable on disk and pushed.
