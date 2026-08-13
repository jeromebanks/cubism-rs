# Time-Series Phase 7 Handoff

Date: 2026-08-12

Branch: `feature/timeseries-phase-0a`

Status: **Docs-only slice — no code changed.** This session's slice was
issue [#15](https://github.com/jeromebanks/cubism-rs/issues/15): write
`docs/TIMESERIES_ROADMAP.md`, decomposing Phase 4 proper (#13) into six
ordered, slice-sized milestones, and point
`.claude/skills/timeseries-slice/SKILL.md`'s step 1 at it. `cargo test`/
`clippy -D warnings` re-run clean across `cubism-iceberg`, `cubism-cli`, and
the full workspace to confirm no regression — nothing here was expected to
change test behavior, and nothing did. Committed and pushed to
`feature/timeseries-phase-0a` — see "Worktree state" below.

(Despite the filename, this doc documents a session slice, not "Phase 7" of
the implementation plan — same convention every prior handoff in this series
has used: plan Phase 5 is DataFusion range queries and serving, plan Phase 4
hasn't started coding yet. This particular slice is unusual in the series in
that it produced no plan-phase progress at all — it produced the sequencing
doc that the *next* several slices will use to make that progress.)

**Superseded by:** [`TIMESERIES_PHASE_8_HANDOFF.md`](TIMESERIES_PHASE_8_HANDOFF.md),
which picked up this doc's deferred item 2 (Milestone 1, `LatenessPolicy`)
using the roadmap this session wrote — the roadmap's first milestone,
implemented and closed.

## What this session built

Read `docs/TIMESERIES_PHASE_6_HANDOFF.md` and confirmed its commit was
pushed and the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`). Read all open issues (#1-#15); #15 — filed after
the Phase 6 session, not itself in Phase 6's deferred list — asks for
exactly this: the Phase 6 handoff's deferred item 6 noted that the remaining
Phase 4 test-list requirements all need `LatenessPolicy`/`CorrectionPlan`/
`ReconciliationRecord`/coordinator (#13) to exist first, and are "not
independently attemptable as further thin test-only slices" without some
sequencing. Consulted the advisor before drafting, which confirmed #15 as
the slice (newest, user-filed, explicitly a prerequisite to further slices,
and the only candidate not too large, a standing deliberate tradeoff, or a
decision not mine to make) and set four constraints: read plan lines 582-670
before drafting anything, bound this slice to Phase 4 only (not also
Phase 5), write for a cold future session rather than a human reader, and
keep this handoff honest about being docs-only.

- **`docs/TIMESERIES_ROADMAP.md`** (new): six milestones
  (`LatenessPolicy` → formalize `ExpectedRevision` + correction-shaped
  stale-rejection test → `CorrectionPlan` → coordinator/job API +
  late-event-rebuild test → `ReconciliationRecord` + failure-injection test
  → public correction API), each with target file(s), the one test it adds,
  its dependencies, and its done-condition. Also states a "Phase 4 done"
  condition distinct from "all milestones done" (plan's completion criteria,
  unresolved decisions, and rollback point each need explicit confirmation,
  not just milestone completion), and a "when the roadmap is exhausted, fall
  back to the deferred-list scan" instruction for the skill.
- **`.claude/skills/timeseries-slice/SKILL.md`** (modified, step 1): now
  directs a session to check the roadmap for the current phase before
  falling back to its original ad hoc deferred-list/issue scan. This slice
  edited the skill that's driving it — the skill's own scope line says
  "cubism-iceberg and its docs," and this change is docs (a roadmap doc plus
  the skill's own workflow doc), consistent with that scope rather than an
  exception to it.

## What was actually verified

No new test — the test count is unchanged from Phase 6 (21 in
`cubism-iceberg`). The full verification battery below was re-run to confirm
this slice introduced no regression, not to verify any new claim; a
docs-only change has nothing else for that battery to check. The roadmap's
factual claims about current code state (`ExpectedRevision`'s existing CAS
mechanism, `StaleRevision`'s existing test coverage, and the absence of
`LatenessPolicy`/`CorrectionPlan`/`ReconciliationRecord`/`CompactionPlan`/
`RetentionPlan`/`WindowLease` anywhere in `crates/cubism-iceberg/src` or
`crates/cubism-core/src`) were checked directly — `Read` on
`crates/cubism-iceberg/src/control.rs` lines 188 and 366-395, and
`rtk proxy grep -rn` for each type name across both crates' `src/`
returning zero matches — not assumed from the issue bodies alone.

## GitHub issues touched

- **#15**: not closed (deliberately — see "Deferred" below; the Phase 5
  extension it also asks for is still outstanding, and the skill
  pre-authorizes commit/push but not issue-state changes). Commented with a
  pointer to this handoff and the new roadmap doc.
- **#7-#14**: untouched — this slice was planning-only and found nothing new
  against any of them.

## Deferred / not done this session

1. **Extending the roadmap to plan-Phase 5** (DataFusion range queries and
   serving). #15's suggested steps ask for this too; the advisor's scoping
   constraint for this session was Phase 4 only, so Phase 5 sequencing
   remains undone. Next slice that reaches the end of the Phase 4 milestone
   list (or picks this up directly) should draft it.
2. **Milestone 1 (`LatenessPolicy`)** itself — the roadmap is written, no
   milestone has been implemented yet. This is the next code slice per the
   roadmap's own step-1 instructions.
3. **The "Rollback point" milestone gap** the roadmap flags under "Phase 4
   done": whether plan lines 666-669 (repoint a window to its prior
   published revision) falls out of Milestone 6's public API or needs its
   own bounded milestone is an open question the roadmap raises but does not
   resolve.
4. Everything already deferred as of Phase 6 (#10/#8/#9/#11/#12/#14) is
   unchanged — this session did not touch any of that scope.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log` for
the exact hash — this doc deliberately doesn't hardcode it, same convention
this series adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a
follow-up commit to fix a self-referential hash). That commit contains:

- New: `docs/TIMESERIES_ROADMAP.md`, `docs/TIMESERIES_PHASE_7_HANDOFF.md`
  (this file).
- Modified: `.claude/skills/timeseries-slice/SKILL.md` (step 1 now consults
  the roadmap first), `docs/TIMESERIES_PHASE_6_HANDOFF.md` (added
  `**Superseded by:**` line), `docs/handoff_latest.md` (symlink repointed).
- Untouched: everything under `crates/` — no source or test files changed
  this session.

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (21 in `cubism-iceberg`: 8 unit + 6 Phase-3 integration + 3 durability integration + 4 concurrency integration, all passing — unchanged from Phase 6, no new tests this session; unit/suite breakdown re-verified this session via raw `cargo test` output, not carried over unchecked)

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 21 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 3 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 157 passed, 1 ignored (unchanged from Phase 6)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (new this session)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (Phase 4 spec, lines 582-670 — the roadmap's source)
- [`TIMESERIES_PHASE_6_HANDOFF.md`](TIMESERIES_PHASE_6_HANDOFF.md)
- [`../.claude/skills/timeseries-slice/SKILL.md`](../.claude/skills/timeseries-slice/SKILL.md)
  (step 1 updated)
- GitHub issue [#15](https://github.com/jeromebanks/cubism-rs/issues/15)
  (this session's slice)
- GitHub issue [#13](https://github.com/jeromebanks/cubism-rs/issues/13)
  (Phase 4 proper, decomposed by the new roadmap)
