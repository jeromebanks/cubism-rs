# Time-Series Phase 29 Handoff

Date: 2026-08-19

Branch: `feature/timeseries-phase-0a`

Status: **Fixed [issue #20](https://github.com/jeromebanks/cubism-rs/issues/20)**
(`CorrectionCoordinator::execute` could silently undo an intentional
rollback via a stale request replay — a classic CAS ABA bug). With the
roadmap's tracked milestone list already exhausted (18/18, per
`docs/TIMESERIES_PHASE_28_HANDOFF.md`), this slice falls back to the
roadmap's own documented behavior for that state: scan open issues and the
latest handoff's deferred list directly. (Naming note, carried forward
from every prior handoff in this series: this file's number is a
*session-slice* number, not a `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`
phase number — this slice does issue-fix work, not plan-phase or
roadmap-milestone work.)

`docs/TIMESERIES_PHASE_28_HANDOFF.md`'s deferred list named `#20` as the
top item — the most consequential open item, unblocked, and already
issue-scoped with the design space enumerated in `#20` itself. Advisor
review (before any writing) confirmed the pick and confirmed the
mechanism read (a classic ABA: `current` goes `A -> B -> A`, so a stale
replay's CAS check can't distinguish "nothing happened" from "changed
then changed back") and the chosen fix direction — reject a `Published`
run whose revision is no longer `current` with a distinct error, rather
than either silently re-publishing or returning a stale `Publication`.

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

**`crates/cubism-iceberg/src/coordinator.rs`** — `CorrectionCoordinator::execute`
(guard added at line 371, just before the final `publications.publish`
call at line 383) now live-checks, when the run's `RunState` is already
`Published`, whether `PublicationStore::current` still equals that run's
own revision before falling through to the CAS publish call. If something
else (a later correction, or a rollback) has since moved `current` away
from it, `execute` returns the new `CubismIcebergError::RunNoLongerCurrent`
error instead of blindly re-publishing. The check reads `current` directly
rather than calling `RunInspection::inspect`, since `execute` already
holds the `RunState` `claim_run` returned and re-reading it through
`inspect` risks observing a different one. `ReconciliationRecord::Published`'s
own doc comment and `execute`'s doc comment were both updated to record
that this gap is now closed for the `Published` case specifically — not
for `AwaitingPublish` (see "GitHub issues touched" below).

**`crates/cubism-iceberg/src/error.rs`** — new
`CubismIcebergError::RunNoLongerCurrent { run_id, window_id, revision,
current: Option<u64> }` variant (line 65). Deliberately not named around
"superseded" (`RevisionStatus`'s own doc comment in `coordinator.rs`
already avoids that word — a not-current revision can be numerically
*higher* than the window's live one after a rollback) and deliberately not
a reuse of `StaleRevision`, whose `expected`/`actual` fields would print
as equal values in exactly the bug scenario this guards against, which
would read as nonsense.

**`crates/cubism-iceberg/tests/coordinator.rs`** — two new integration
tests:

1. `coordinator_rejects_a_replayed_correction_after_a_rollback_restored_its_observed_current`
   (line 475) — reproduces `#20`'s exact bug shape end to end: publish
   revision A, correct to revision B (moving `current` to B), roll back to
   A via the existing CAS `publish` mechanism
   (`docs/TIMESERIES_PHASE_13_HANDOFF.md`, not new code), then replay the
   *original* correction request (still carrying `observed_current: A`
   from its first submission). Asserts the replay now returns
   `RunNoLongerCurrent { revision: 2, current: Some(1), .. }` instead of
   silently republishing B, and that `current` is still A afterward.
2. `coordinator_replaying_a_published_correction_with_nothing_changed_returns_the_same_publication`
   (line 406) — the benign side: replaying `execute` against an
   already-`Published` run when nothing else has touched `current` must
   still return the identical `Publication` (same `revision` and
   `aggregate_snapshot_id`), preserving Milestone 5's idempotent-replay
   contract. No existing integration test replayed `execute` against an
   already-`Published` run before this session — the closest prior test,
   `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`,
   leaves the run `Appended`, not `Published` — so this is new coverage,
   not a re-proof of something already covered.

## What was actually verified

Both new tests pass (`cargo test -p cubism-iceberg --test coordinator`:
5 passed, up from 3). The bug-repro test independently drives the exact
sequence `#20`'s issue body describes (claim/append/publish for the
correction, then a second `publish` call replaying the rollback, then a
second `execute` call replaying the original request) rather than calling
into any helper that could hide a step — every stage is a direct call
against `fixture.publications`/`CorrectionCoordinator::execute`. The
benign-replay test's assertion is on both `revision` and
`aggregate_snapshot_id` specifically (not just success/failure), so it
would catch a fix that accidentally performed a second append or a
different publish path on replay, not just one that happened to error.

**What this does not prove:**

- This fix is scoped to a run whose `RunState` is `Published` at replay
  time. The `AwaitingPublish` case (a run that crashed *before* its own
  first `publish` call, then gets replayed after `current` has cycled back
  to the value it originally observed) is a distinct, deliberately
  out-of-scope gap — filed as
  [#21](https://github.com/jeromebanks/cubism-rs/issues/21), not
  mechanically the same fix (see that issue for why: for an
  `AwaitingPublish` run, "is my revision current" is always false before
  its own first publish, so the live-current equality check this fix uses
  doesn't transfer as written).
- No test exercises the `RunNoLongerCurrent` error path against the
  *other* way a `Published` run can go not-current — a later correction
  advancing `current` past it (as opposed to a rollback moving it back) —
  though the guard's logic (`current != Some(revision)`, unconditional on
  *why*) covers both by construction; only the rollback shape has direct
  test coverage.
- This does not re-verify concurrent-recovery-of-the-same-run (already
  explicitly out of scope per `#17`'s own non-goal, unrelated to this fix).

Full-suite counts, both re-verified this session: `cargo test -p
cubism-iceberg` reports **37 passed, 1 ignored (6 suites)** — up from 35
in Phase 28, matching the two new tests exactly (13 unit + 4 concurrency +
5 coordinator + 9 durability + 6 phase3 + 0 doctests). `cargo test
--workspace --exclude cubism-py` reports **213 passed, 2 ignored (24
suites)** — up from 211, same delta. `rustfmt --edition 2024 --check` on
every touched `.rs` file reported pre-existing diffs unrelated to this
session's lines (confirmed already tracked as
[#3](https://github.com/jeromebanks/cubism-rs/issues/3)) — per the skill's
own step-2 note, plain `rustfmt` was **not** run on any touched file; new
lines were hand-written to match the surrounding wide-line style instead,
and `cargo clippy -D warnings` (which doesn't care about formatting)
passed clean on every touched crate.

## GitHub issues touched

- **[#20](https://github.com/jeromebanks/cubism-rs/issues/20)** — fixed
  this session (see above).
- **[#21](https://github.com/jeromebanks/cubism-rs/issues/21)** — filed
  this session: the `AwaitingPublish` variant of the same ABA shape,
  found by advisor while scoping `#20`'s fix, deliberately left out of
  this slice's bounded scope. Needs its own design pass (see the issue for
  why it isn't a mechanical port of this fix).

## Deferred / not done this session

1. **`#21`** (`AwaitingPublish` variant of the ABA replay gap) — newly
   filed this session, unblocked, needs a design decision the same way
   `#20` did before this session's fix could be written.
2. **The roadmap's tracked milestone list remains exhausted (18/18)** —
   unchanged by this slice, which touched no roadmap milestone. The next
   slice should keep falling back to the open-issue/deferred-list scan
   (per `docs/TIMESERIES_ROADMAP.md`'s own step-1 guidance, lines 52-64)
   until a new milestone is added to the roadmap. `#21` (above) or
   `#10` (compaction/retention/object-store, the other long-standing
   top-of-list item) are both live candidates.
3. Every item on `docs/TIMESERIES_PHASE_28_HANDOFF.md`'s deferred list
   items 2-3 (`#10`, `#9`, `#8`, `#16`, `#14`, `#4`, `#3` — all still
   open, all unchanged this session) and the TOCTOU/concurrent-request
   items further back in this series' history — unchanged, none touched
   this session.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention).

- Modified: `crates/cubism-iceberg/src/coordinator.rs` (the `#20` fix and
  updated doc comments), `crates/cubism-iceberg/src/error.rs` (new
  `RunNoLongerCurrent` variant), `crates/cubism-iceberg/tests/coordinator.rs`
  (two new integration tests), `docs/TIMESERIES_PHASE_28_HANDOFF.md`
  (`Superseded by` line added), `docs/handoff_latest.md` (symlink
  retarget).
- New: `docs/TIMESERIES_PHASE_29_HANDOFF.md` (this file).
- Untouched: `docs/TIMESERIES_ROADMAP.md` (this slice is issue work, not
  milestone work — see "Status" above), every other crate.

Also present, deliberately uncommitted, prior-session convention, left
untouched this session: `.serena/` (local tooling state);
`examples/web_analytics_demo/events.csv` (the *static* demo's own
generated output, unrelated to this session);
`.claude/skills/timeseries-slice/SKILL.md` (pre-existing uncommitted edits
from before this session started, confirmed unrelated to this slice's
scope by reading `docs/TIMESERIES_PHASE_28_HANDOFF.md`'s own note about
the same file).

## Tests (37 passed + 1 ignored in `cubism-iceberg`, up from 35; 213 passed / 2 ignored in workspace, up from 211)

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 37 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                            # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                             # 9 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings      # clean
cargo build -p cubism-cli                                                  # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings         # clean
cargo test --workspace --exclude cubism-py                                 # 213 passed, 2 ignored (24 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

Every leg's full output was captured to a file first and grepped for
`test result:`/`error` afterward, not piped through `tail` directly, per
the skill's own step-4 note about a truncated capture producing a wrong
total before. The workspace total (213) was summed by hand from every
suite's own `test result:` line in the captured log, not read off a
single truncated tail.

## Primary files

- [`coordinator.rs`](../crates/cubism-iceberg/src/coordinator.rs) (the
  `#20` fix)
- [`error.rs`](../crates/cubism-iceberg/src/error.rs) (`RunNoLongerCurrent`)
- [`tests/coordinator.rs`](../crates/cubism-iceberg/tests/coordinator.rs)
  (the two new tests)
- [`TIMESERIES_PHASE_28_HANDOFF.md`](TIMESERIES_PHASE_28_HANDOFF.md)
  (prior handoff, superseded by this one)
- GitHub issue [`#20`](https://github.com/jeromebanks/cubism-rs/issues/20)
  (fixed this session)
- GitHub issue [`#21`](https://github.com/jeromebanks/cubism-rs/issues/21)
  (filed this session, deferred)
