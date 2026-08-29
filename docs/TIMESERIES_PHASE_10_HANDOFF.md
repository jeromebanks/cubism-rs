# Time-Series Phase 10 Handoff

Date: 2026-08-14

Branch: `feature/timeseries-phase-0a`

Status: **Milestone 3 (`CorrectionPlan` strategy selection) landed and
closed.** New module `crates/cubism-iceberg/src/correction.rs` with
`CorrectionPlan::select` and three unit tests, full step-4 verification
battery clean. `docs/TIMESERIES_ROADMAP.md` updated: Milestone 3 marked
`Done`, its "What it does" scope narrowed in place to match what was
actually built, and its "Depends on Milestone 1" line corrected — that
dependency never became a real code dependency. Nothing else in Phase 4's
scope moved this session — Milestones 1-2 (already done), 4-6 untouched.

(Despite the filename, this doc documents a session slice, not "Phase 10"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving; plan
Phase 4 is now three milestones into its six-milestone roadmap. See
`docs/TIMESERIES_ROADMAP.md` for what "Milestone 3" means relative to plan
phases.)

**Superseded by:** `docs/TIMESERIES_PHASE_11_HANDOFF.md`, which lands
Milestone 4 (narrowed — see that doc for why "identifies affected windows
from event time" and the literal rebuild-equality test are split out into
[#16](https://github.com/jeromebanks/cubism-rs/issues/16) rather than
completed as originally worded).

## What this session built

Read `docs/TIMESERIES_PHASE_9_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and read all 15 open issues. Per the roadmap's
step-1 instructions, checked the milestone list first: Milestones 1 and 2
were already done, Milestone 3 was the first not-done milestone with its
only listed dependency (Milestone 1) done — the unambiguous pick.

Consulted the advisor before implementing. The advisor flagged a real
tension worth resolving before writing code: the roadmap's stated target
for Milestone 3 is `crates/cubism-iceberg/src`, but `LatenessPolicy` and
`capabilities_for`/`AggregateCapabilities` both live in `cubism-core`, and
Phase 9's session had explicitly kept `LatenessPolicy` out of
`cubism-iceberg` code to avoid reopening a cross-crate scope question.
Checking `crates/cubism-iceberg/Cargo.toml` resolved it: `cubism-iceberg`
already depends on `cubism-core` (line 10), so consuming
`capabilities_for` from `cubism-iceberg/src/correction.rs` is not a new
crate dependency — the roadmap's stated target was followed as written, no
deviation needed.

The advisor also caught an important precision issue before any test got
written: `AggregateCapabilities`'s two constructors, `exact(associative,
commutative, idempotent, subtractable)` and `floating(commutative,
idempotent, subtractable, tolerance)`
(`crates/cubism-core/src/aggregate_state.rs:71-101`), take positionally
different bools. Reading `capabilities_for`
(`crates/cubism-core/src/aggregate_state.rs:109-160`) with the correct
positions showed that **no current `AggKind` has both `subtractable: true`
and `idempotent: true`** — `Sum` (`:111`) and `Count` (`:112`) are the only
`subtractable: true` kinds, and both are `idempotent: false`. That fact
directly shapes what plan line 635's test can honestly claim: a rule
requiring both flags is well-supported by the codebase's own "false means
don't use this for corrections" doc comment on `AggregateCapabilities`, but
it also means the `AdditiveShortcut` branch is not reachable by any real
`AggKind` today — the tests say this explicitly rather than implying wider
coverage.

The advisor's third point — whether `CorrectionPlan` needs `LatenessPolicy`
at all for line 635's test — was answered no: the test is purely about
aggregate-kind strategy selection, no timestamps involved. `correction.rs`
does not reference `LatenessPolicy`, and the roadmap's "Depends on
Milestone 1" line is corrected to say so.

- **`crates/cubism-iceberg/src/correction.rs`** (new): `CorrectionStrategy`
  (`FullRebuild` | `AdditiveShortcut`, line 23) and `CorrectionPlan`
  (line 45) with `CorrectionPlan::select(kinds: &[AggKind])` (line 53),
  which grants `AdditiveShortcut` only when every kind in `kinds` has both
  `subtractable` and `idempotent` true, and treats an empty slice as
  ineligible. Three tests:
  `full_rebuild_is_selected_for_every_current_agg_kind` (line 88) checks
  each of the 11 current `AggKind` variants individually plus the full set
  together, all refused a shortcut;
  `shortcut_requires_both_subtractable_and_idempotent` (line 122) uses
  `Sum` (subtractable, not idempotent) to prove the rule is the conjunction
  of both flags, not `subtractable` alone;
  `empty_kind_list_selects_full_rebuild` (line 134) covers the
  vacuous-`all()` edge case explicitly rather than leaving it as an
  implicit consequence of the `all()` implementation. Each test's doc
  comment states plainly that no current `AggKind` reaches the
  `AdditiveShortcut` branch — it's reachable in principle, not exercised by
  any real kind yet.
- **`crates/cubism-iceberg/src/lib.rs`** (modified): added `pub mod
  correction;` and `pub use correction::{CorrectionPlan,
  CorrectionStrategy};`, following the existing pattern for `control`.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 3 marked `Done —
  docs/TIMESERIES_PHASE_10_HANDOFF.md`; "What it does" narrowed in place to
  describe what was actually built (strategy selection only — no source
  checkpoint, time range, or affected-windows representation, since nothing
  in this crate consumes those yet; that's Milestone 4's coordinator's job);
  "Depends on Milestone 1" corrected — `LatenessPolicy` was never a code
  dependency of this milestone's test.

## What was actually verified

The new tests prove: (1) for every `AggKind` this workspace currently
defines, `CorrectionPlan::select` chooses `FullRebuild`, both for each kind
individually and for the full set of all eleven kinds together; (2) the
refusal rule is genuinely "`subtractable` AND `idempotent`," not
`subtractable` alone — demonstrated using `Sum`, which is `subtractable`
but not `idempotent` and is still refused a shortcut; (3) an empty kind
list is treated as ineligible rather than vacuously eligible. They do
**not** prove that `AdditiveShortcut` is ever actually selected for a real
`AggKind` — none currently qualifies, so that branch is only reachable via
the type's public API, never exercised by production data through this
test suite. They also do not prove anything about `LatenessPolicy`,
`ReconciliationRecord`, a real coordinator, or actual correction execution
(subtracting a stale contribution and applying a corrected one) — those
remain Milestones 4-6's scope. `CorrectionPlan` does not yet carry a source
checkpoint, time range, or affected-windows list; this session deliberately
scoped it to strategy selection only, per the advisor's guidance to avoid
building untested scaffolding ahead of the coordinator that will actually
consume it.

## GitHub issues touched

None. This session's whole scope was Milestone 3 as scoped by the roadmap
(narrowed in place, not re-scoped) — no newly-discovered deferred item
emerged that isn't already tracked by an existing issue or the roadmap's
own milestone list. Checked Phase 9's deferred list against the roadmap and
all 15 open issues per skill step 0/3, including reading issue #9's full
body (aggregate state blob checksum framing) to confirm it does not overlap
`CorrectionPlan`'s scope — it doesn't; #9 is about blob-level corruption
detection, unrelated to correction strategy selection.

- Deferred item 1 (Milestone 3) is this session's own code slice.
- Deferred item 2 (Milestones 4-6) unchanged — still future slices, not
  issues to file individually (the roadmap already tracks them).
- Deferred item 3 (extending the roadmap to plan-Phase 5) unchanged, still
  the roadmap's own "Deferred" section item.
- Deferred item 4 (the rollback-point milestone gap) unchanged, still an
  open question flagged in the roadmap's "Phase 4 done" section, not
  actionable independent of Milestone 6.
- Deferred item 5 (`#10`/`#8`/`#9`/`#11`/`#12`/`#14`) unchanged, no new
  findings against any of them this session.

## Deferred / not done this session

1. **Milestone 4** (Coordinator/job API + late-event rebuild test) — the
   roadmap's next milestone, depends on Milestone 2 (done) and Milestone 3
   (done this session). This is the next code slice per the roadmap. It
   will need to give `CorrectionPlan` the source checkpoint/time
   range/affected-windows representation this session deliberately left
   out.
2. **Milestones 5-6** — untouched, each still depends on earlier milestones
   not yet done.
3. **Extending the roadmap to plan-Phase 5** — still deferred, unchanged
   from Phase 9.
4. **The rollback-point milestone gap** — still an open question the
   roadmap flags under "Phase 4 done," not resolved this session.
5. Everything already deferred as of Phase 9 (`#10`/`#8`/`#9`/`#11`/`#12`/
   `#14`) is unchanged — this session did not touch any of that scope.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, same
convention adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a
follow-up commit to fix a self-referential hash). That commit contains:

- New: `crates/cubism-iceberg/src/correction.rs` (new module, three
  tests), `docs/TIMESERIES_PHASE_10_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/src/lib.rs` (module wiring),
  `docs/TIMESERIES_ROADMAP.md` (Milestone 3 marked done, scope narrowed,
  dependency corrected), `docs/TIMESERIES_PHASE_9_HANDOFF.md` (added
  `**Superseded by:**` line), `docs/handoff_latest.md` (symlink
  repointed).
- Untouched: `crates/cubism-core/`, `crates/cubism-cli/`, and everything
  else under `crates/cubism-iceberg/src/` except `correction.rs` and
  `lib.rs` — `control.rs`, `durable_control.rs`, and the rest of the crate
  are unchanged this session.

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (25 in `cubism-iceberg`, +3 this session; 162 in workspace, +3)

`cargo test -p cubism-iceberg` reports 25 passed (5 suites): 11 unit
(`--lib`, +3 this session, was 8) + 6 Phase-3 integration (`--test
phase3`) + 4 durability integration (`--test durability`, unchanged) + 4
concurrency integration (`--test concurrency`, unchanged). All four
figures were confirmed by running each suite in isolation (`--lib`,
`--test phase3`, `--test durability`, `--test concurrency` each
individually) during the advisor follow-up pass — not derived by
subtraction from the workspace total, which the first draft of this doc
claimed without having actually run `--lib`/`--test phase3` in isolation
yet. `cargo
test --workspace --exclude cubism-py` reports 162 passed, 1 ignored (21
suites) — a +3 from Phase 9's 159, matching this session's three new
tests; `cubism-core`'s 92 tests are untouched (no `cubism-core` source
changed this session).

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 25 passed (5 suites)
cargo test -p cubism-iceberg --lib                                        # 11 passed
cargo test -p cubism-iceberg --test phase3                                # 6 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 4 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 162 passed, 1 ignored (21 suites; +3 from Phase 9's 159)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-iceberg/src/correction.rs`](../crates/cubism-iceberg/src/correction.rs)
  (new module: `CorrectionPlan`, `CorrectionStrategy`, three tests)
- [`../crates/cubism-iceberg/src/lib.rs`](../crates/cubism-iceberg/src/lib.rs)
  (module wiring)
- [`../crates/cubism-core/src/aggregate_state.rs`](../crates/cubism-core/src/aggregate_state.rs)
  (`capabilities_for`, lines 109-160 — the data this milestone's strategy
  rule reads; `exact`/`floating` constructors, lines 71-101 — the
  positional-argument precision the advisor flagged)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 3 marked
  done, scope narrowed, dependency corrected)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (plan line 635, the test this milestone closes)
- [`TIMESERIES_PHASE_9_HANDOFF.md`](TIMESERIES_PHASE_9_HANDOFF.md)
- GitHub issue [`#13`](https://github.com/jeromebanks/cubism-rs/issues/13)
  (Phase 4 proper, this milestone's parent scope)
- GitHub issue [`#15`](https://github.com/jeromebanks/cubism-rs/issues/15)
  (roadmap origin)
