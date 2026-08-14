# Time-Series Phase 9 Handoff

Date: 2026-08-14

Branch: `feature/timeseries-phase-0a`

Status: **Milestone 2 (formalize `ExpectedRevision`, correction-shaped
stale-rejection test) landed and closed.** New integration test
`sqlite_correction_against_a_superseded_revision_is_rejected_then_succeeds_on_retry`
in `crates/cubism-iceberg/tests/durability.rs`, full step-4 verification
battery clean. `docs/TIMESERIES_ROADMAP.md` updated: Milestone 2 marked
`Done`, and the milestone's original "distinct from
`stale_publish_cannot_replace_a_newer_revision`" justification — which the
advisor caught was factually wrong — corrected in place. Nothing else in
Phase 4's scope moved this session — Milestones 1 (already done), 3-6
untouched.

(Despite the filename, this doc documents a session slice, not "Phase 9" of
the implementation plan — same convention every prior handoff in this series
has used: plan Phase 5 is DataFusion range queries and serving; plan Phase 4
is now two milestones into its six-milestone roadmap. See
`docs/TIMESERIES_ROADMAP.md` for what "Milestone 2" means relative to plan
phases.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_8_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and read all 15 open issues. Per the roadmap's
step-1 instructions, checked the milestone list first: Milestone 1 was
already done, Milestone 2 was the first not-done milestone with no
unmet dependencies — the unambiguous pick.

Consulted the advisor before implementing. The advisor caught that the
roadmap's own spec for Milestone 2's test was, step for step, already
implemented by `control.rs`'s existing
`stale_publish_cannot_replace_a_newer_revision` test — and that the
roadmap's stated reason the new test would be "distinct" (that the existing
one "races two initial runs") was itself wrong: the test that races two
initial runs against `expected: None` is `tests/concurrency.rs`'s
`two_same_window_writers_produce_exactly_one_published_winner`
(`concurrency.rs:63`, assertions at `:111`/`:115`), not
`stale_publish_cannot_replace_a_newer_revision`. Writing the roadmap's
literal spec would have shipped a renamed duplicate — exactly the failure
mode this series' "state what a test does and does not prove" convention
exists to catch.

The advisor identified the actual untested intersection instead: no
existing test ran the superseded-revision CAS shape against the **durable**
(SQLite) backend with a genuinely-superseded `expected` value.
`durability.rs`'s only CAS-reject test,
`sqlite_publication_store_cas_survives_reopen_from_a_fresh_handle`, asserts
against `Some(99)` — a value that was never valid, so it never exercises
recovery from a real superseded belief. Nor does any existing test carry a
rejected publish through refresh-`current`-then-retry to success, the
protocol Milestone 4's coordinator will need. That's the gap this session's
test fills.

- **`crates/cubism-iceberg/tests/durability.rs`** (modified): added
  `sqlite_correction_against_a_superseded_revision_is_rejected_then_succeeds_on_retry`.
  Sequence: run-1 publishes (rev1, `expected: None`); run-2 supersedes it
  (rev2, `expected: Some(rev1)`); run-3 (the correction) attempts to publish
  with `expected: Some(rev1)` — a belief that *was* genuinely current when
  the correction started, unlike the reopen test's never-valid `99` — and is
  rejected with `StaleRevision { expected: Some(1), actual: Some(2), .. }`;
  `current` is asserted unchanged; run-3 then re-reads `current`, retries
  with the refreshed value, succeeds, and the result is confirmed visible
  from a third, freshly-opened handle. Doc comment states plainly what
  distinguishes this test from the two nearest existing ones and explicitly
  disclaims exercising `LatenessPolicy` or any coordinator/`CorrectionPlan`
  machinery — those don't exist yet (Milestones 3-4). Per the advisor's
  explicit instruction, `LatenessPolicy` is referenced only in the doc
  comment's framing prose, not wired into any `cubism-iceberg` code — doing
  so would have re-opened the cross-crate scope question Phase 8 already
  had to resolve, for no test behavior gained.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 2 marked
  `Done — docs/TIMESERIES_PHASE_9_HANDOFF.md`; target corrected to
  `durable_control.rs` (confirmed its `publish` has the same
  already-current-early-return-then-CAS-then-write ordering as
  `control.rs:207-217`, at `durable_control.rs:297-334`); the milestone's
  own "distinct from `stale_publish_cannot_replace_a_newer_revision`"
  justification rewritten in place with the correct reasoning (per this
  series' convention of correcting live docs in place, as opposed to
  handoffs, which only get additive pointers); test name and behavior
  recorded; plan line 660's decision ("lease service versus optimistic
  expected-revision only," verified via `rtk proxy grep -n "" ... | awk`
  against `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` — confirmed at line 660
  exactly, not approximated) recorded as resolved in favor of the
  already-built optimistic mechanism.

## What was actually verified

The new test proves: (1) a publish whose `expected_current` matches a
revision that *was* genuinely current, but has since been superseded by
another publish, is rejected via the durable (SQLite) `StaleRevision` path
with the correct `expected`/`actual` pair, and does not move `current`; (2)
the same run can then refresh its belief via `current` and retry to a
successful publish; (3) that successful publish is visible from an
independently-opened third handle, not just the retrying handle's own
in-memory state. It does **not** prove anything about `LatenessPolicy`,
a real coordinator, `CorrectionPlan`, or concurrent (as opposed to
sequential) attempts at the retry step — those remain future milestones'
scope. It also does not re-verify the in-memory backend's equivalent
behavior (`control.rs`'s existing `stale_publish_cannot_replace_a_newer_revision`
test already covers that path and was left untouched).

## GitHub issues touched

None. This session's whole scope was Milestone 2 as scoped by the roadmap
(corrected in place, not re-scoped) — no newly-discovered deferred item
emerged that isn't already tracked by an existing issue or the roadmap's own
milestone list. Checked Phase 8's deferred list against the roadmap and all
15 open issues per skill step 0/3:

- Deferred item 1 (Milestone 2) is this session's own code slice.
- Deferred item 2 (Milestones 3-6) unchanged — still future slices, not
  issues to file individually (the roadmap already tracks them).
- Deferred item 3 (extending the roadmap to plan-Phase 5) unchanged, still
  the roadmap's own "Deferred" section item.
- Deferred item 4 (the rollback-point milestone gap) unchanged, still an
  open question flagged in the roadmap's "Phase 4 done" section, not
  actionable independent of Milestone 6.
- Deferred item 5 (`#10`/`#8`/`#9`/`#11`/`#12`/`#14`) unchanged, no new
  findings against any of them this session.

## Deferred / not done this session

1. **Milestone 3** (`CorrectionPlan`) — the roadmap's next milestone,
   depends on Milestone 1 (done). This is the next code slice per the
   roadmap.
2. **Milestones 4-6** — untouched, each still depends on earlier milestones
   not yet done.
3. **Extending the roadmap to plan-Phase 5** — still deferred, unchanged
   from Phase 8.
4. **The rollback-point milestone gap** — still an open question the
   roadmap flags under "Phase 4 done," not resolved this session.
5. Everything already deferred as of Phase 8 (`#10`/`#8`/`#9`/`#11`/`#12`/
   `#14`) is unchanged — this session did not touch any of that scope.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log` for
the exact hash — this doc deliberately doesn't hardcode it, same convention
adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a follow-up commit
to fix a self-referential hash). That commit contains:

- New: `docs/TIMESERIES_PHASE_9_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/tests/durability.rs` (new test),
  `docs/TIMESERIES_ROADMAP.md` (Milestone 2 marked done, corrected
  justification, plan-line-660 decision recorded),
  `docs/TIMESERIES_PHASE_8_HANDOFF.md` (added `**Superseded by:**` line),
  `docs/handoff_latest.md` (symlink repointed).
- Untouched: `crates/cubism-core/`, `crates/cubism-cli/`, and everything
  else under `crates/cubism-iceberg/src/` — no library source changed this
  session, only a test file.

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (22 in `cubism-iceberg`, +1 this session; 159 in workspace, +1)

`cargo test -p cubism-iceberg` reports 22 passed (5 suites): 8 unit
(`--lib`) + 6 Phase-3 integration (`--test phase3`) + 4 durability
integration (`--test durability`, +1 this session, was 3) + 4 concurrency
integration (`--test durability`, unchanged), each isolated and confirmed
directly rather than derived by subtraction alone. `cargo test --workspace
--exclude cubism-py` reports 159 passed, 1 ignored (21 suites) — a +1 from
Phase 8's 158, matching this session's one new test; `cubism-core`'s 92
tests are untouched (no `cubism-core` source changed this session).

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 22 passed (5 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 4 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 159 passed, 1 ignored (21 suites; +1 from Phase 8's 158)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-iceberg/tests/durability.rs`](../crates/cubism-iceberg/tests/durability.rs)
  (new test)
- [`../crates/cubism-iceberg/src/durable_control.rs`](../crates/cubism-iceberg/src/durable_control.rs)
  (`publish`, lines 260-339 — the code path the new test exercises)
- [`../crates/cubism-iceberg/src/control.rs`](../crates/cubism-iceberg/src/control.rs)
  (`stale_publish_cannot_replace_a_newer_revision`, the in-memory analogue
  this test's durable+refresh-retry coverage extends)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 2 marked done,
  justification corrected)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (plan line 660, the "lease vs. optimistic" decision this milestone
  resolves; line 639, the test this milestone closes)
- [`TIMESERIES_PHASE_8_HANDOFF.md`](TIMESERIES_PHASE_8_HANDOFF.md)
- GitHub issue [`#13`](https://github.com/jeromebanks/cubism-rs/issues/13)
  (Phase 4 proper, this milestone's parent scope)
- GitHub issue [`#15`](https://github.com/jeromebanks/cubism-rs/issues/15)
  (roadmap origin)
