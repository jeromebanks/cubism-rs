# Time-Series Phase 4 Cross-Model Review

Date: 2026-08-17

Branch: `feature/timeseries-phase-0a`

Phase: 4 (`docs/TIMESERIES_ROADMAP.md`'s Phase 4 milestone list — late data,
corrections, and the coordinator; `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`
lines 582-670, per `#13`). Landing Milestone 5b (fixing
[#17](https://github.com/jeromebanks/cubism-rs/issues/17)) satisfied Phase
4's own "done" condition for the first time — see
`docs/TIMESERIES_ROADMAP.md`'s "Phase 4 done condition" section for the
criterion-by-criterion walk. This is Phase 4's **first-ever** cross-model
phase review: the step 8a process (`.claude/skills/timeseries-slice/SKILL.md`)
did not exist when Milestones 1-6 landed (Phase 5 was the first phase this
process applied to, per the skill's own "Forward-only" note) — Phase 4 only
reaches it now because this is the first time its "done" condition has
actually been satisfied, not a retroactive re-review of something already
closed.

Diff range: `a253f4f` (`Add handoff_latest.md symlink and timeseries-slice
skill` — the commit immediately before `1836d50` added this roadmap doc's
first "## Milestones" section, Milestone 1; same backfill convention Phase
5's own start, `f0599b2`, used) through `73145b1` (`timeseries: land
Milestone 5b (AwaitingAppend recovery), fix #17, close #18` — the commit
that satisfied Phase 4's "done" condition). This is the entire Phase 4 diff,
Milestones 1 through 5b, not just this session's own slice.

Reviewer: Codex, via this repo's `codex-companion.mjs` runtime
(`node codex-companion.mjs review --wait --json --scope branch --base a253f4f`).
Ran to completion in a normal amount of time for a phase-sized diff (no
fallback needed).

## Full review text (verbatim)

> A delayed correction retry can reverse an intentional rollback, and valid
> extreme lateness values can panic or produce incorrect classifications.
> These are correctness defects in newly added behavior.
>
> Full review comments:
>
> - [P1] Prevent retries from undoing a rollback —
>   `/Users/jeromebanks/dev/cubism_saas/cubism/crates/cubism-iceberg/src/coordinator.rs:330-330`
>   When a correction is already `Published` and its window is later rolled
>   back to `request.observed_current`, replaying the same request skips the
>   append and reaches this unconditional publish. Because the current
>   revision again equals the old CAS anchor, the publish succeeds and
>   silently makes the rolled-back revision current again; treat a
>   published-but-no-longer-current run as terminal or stale instead of
>   republishing it.
>
> - [P2] Avoid overflowing the lateness deadline —
>   `/Users/jeromebanks/dev/cubism_saas/cubism/crates/cubism-core/src/temporal.rs:490-492`
>   When `bucket_end + allowed_lateness` exceeds `i64::MAX`, both inputs are
>   still valid public values, but this addition panics in debug builds and
>   wraps in release builds, potentially classifying an on-time event as
>   late. Perform the comparison in `i128` or use checked arithmetic.

## Dispositions

**[P1] — deferred, filed as
[#20](https://github.com/jeromebanks/cubism-rs/issues/20).** Verified by
reading `coordinator.rs:330` directly: the finding is accurate — that line
predates Milestone 5b (it is `execute`'s original, unconditional final
`publish` call, unchanged by this session's `#17` fix) and the mechanism
described (a replayed request whose `observed_current` coincidentally
matches `current` again after a rollback silently re-publishes a stale run)
is real. Not fixed in this session: unlike P2, this needs a real design
decision (what should `execute` do when a `Published` run's revision is no
longer current — no-op, error, something else — without breaking the
*intended* replay-of-an-unchanged-request idempotency Milestone 5 already
built), not a mechanical arithmetic fix. Same "currently dormant" property
as `#17` had before its own fix: `CorrectionCoordinator::execute` has no
real callers today (confirmed by grep across every crate), so this is a
real, latent correctness gap with zero current blast radius — tracked, not
silently dropped, matching this series' standard treatment of gaps found by
step 8a that need more than "one bounded slice" to resolve correctly (the
same disposition Phase 5's own review gave `#19` before its later,
separate fix).

**[P2] — fixed this session, follow-up commit.** Verified by reading
`temporal.rs:490-492` directly: `LatenessPolicy::classify` computes
`bucket_end.unix_micros() + self.allowed.micros()` as a plain `i64`
addition; `AllowedLateness::from_micros` only rejects negative values (no
upper bound) and `BucketEnd` has no upper bound either, so a `bucket_end`
near `i64::MAX` plus a large `allowed` genuinely overflows — confirmed, not
assumed. Fixed by widening to `i128` (the exact pattern Phase 5's own
review established for the identical class of bug in
`auto_select_resolution`, `crates/cubism-datafusion/src/range_query.rs`);
regression test `lateness_policy_classify_does_not_overflow_on_extreme_inputs`
(`crates/cubism-core/src/temporal.rs`) constructs
`AllowedLateness::from_micros(i64::MAX)` and `BucketEnd::from_unix_micros(i64::MAX)`
together and asserts `EventTime::from_unix_micros(i64::MAX)` classifies
`OnTime` — a case the pre-fix code would panic (debug) or silently
misclassify as `Late` (release) on.

## Notes

Both findings are in code that predates Milestone 5b itself — this session's
own new code (`AggregateReader::run_append_snapshot` and its call sites) was
not flagged. That is expected, not a gap in the review's coverage: this is
Phase 4's *first* review, covering six milestones' worth of accumulated code
in one pass, not a review scoped to one slice's own diff.

Scope caveat: this review ran with `--scope branch --base a253f4f`, i.e.
against the tree at `73145b1`. Two things postdate that tree and so fall
outside what this review actually covered: the P2 fix itself
(`crates/cubism-core/src/temporal.rs`, widened to `i128`) and the
`run_append_snapshot_returns_none_against_a_table_with_no_commits_at_all`
test, both landed in the separate follow-up commit that applies this
review's own dispositions. "The review covered Phase 4" means the
`a253f4f..73145b1` diff, not the tree as it stands after that follow-up
commit — the fix was not re-reviewed, by design (see the roadmap's step 8a
note on not re-running a phase review to chase its own fix commit).
