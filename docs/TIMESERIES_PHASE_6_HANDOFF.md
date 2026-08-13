# Time-Series Phase 6 Handoff

Date: 2026-08-12

Branch: `feature/timeseries-phase-0a`

Status: **Phase 4 proper (late data, corrections, compaction —
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 582-670) still has not
started.** This session took the next bounded slice of Phase 4's own test
list after `docs/TIMESERIES_PHASE_5_HANDOFF.md`'s: "disjoint windows can
commit concurrently" (plan line ~637, immediately after line 636, which the
prior session closed). It also filed the two deferred items that didn't
already have a GitHub issue. `cargo test`/`clippy -D warnings` clean across
`cubism-iceberg`, `cubism-cli`, and the full workspace. Committed and pushed
to `feature/timeseries-phase-0a` — see "Worktree state" below.

## What this session built

Read `docs/TIMESERIES_PHASE_5_HANDOFF.md`, confirmed its commits
(`af7c173`, `2e18eb0`, `d7efc04`) were already pushed and the branch was in
sync with `origin` (`git rev-list --left-right --count` reported `0 0`).
Consulted the advisor before writing any code to scope this session's slice
correctly — in particular, to check issue #10's actual body before filing a
new Phase 4 issue, since its title ("Real object store support and Iceberg
maintenance (compac…") is truncated in list views and turned out to already
cover compaction/retention/orphan cleanup.

- **`disjoint_windows_can_commit_concurrently_without_conflicting`**
  (`crates/cubism-iceberg/tests/concurrency.rs`, new test, 4th in the file):
  two different windows (`2026-08-12`, `2026-08-13`), each claimed/appended
  by its own handle, published behind a `tokio::sync::Barrier`. Asserts both
  `publish` calls return `Ok`, each with the correct `run_id`, and a third
  fresh handle sees each window's own revision uncorrupted by the other's
  concurrent write. See "What was actually verified" below — the name is
  more precise than it might sound.
- **Two GitHub issues filed** for the deferred items that didn't already
  have one — see "GitHub issues touched" below.

## What was actually verified (read this before trusting the test name)

`max_connections(1)` and a real `BEGIN IMMEDIATE` (see
`durable_control.rs`'s module doc comment, unchanged this session) mean
`SqliteStore` takes a database-file-level write lock per transaction, not a
per-window or per-row lock. The new test's two `publish` calls therefore do
**not** execute in parallel — SQLite serializes them at the file level, one
transaction fully completing before the other's begins, same as every other
test in this file. Nothing about that changed this session; the test does
not claim otherwise.

What the test does demonstrate, precisely: racing two transactions against
**unrelated** windows produces zero cross-window interference. Neither
`publish` call is rejected by the other's activity (no spurious
`StaleRevision`/`RunConflict` from a window that isn't actually in
conflict), and each window's published revision is exactly the one its own
writer wrote — a fresh third handle confirms this via `.current()` on both
windows independently. That is the application-level guarantee Phase 4's
line ~637 actually asks for: two unrelated windows don't corrupt each
other, not that the store executes them in parallel. The store's global
serialization (one writer at a time, regardless of which window) is a real
scaling limit worth knowing about before treating this store as a
production concurrency backend — it's exactly the risk `docs/TIMESERIES_PHASE_5_HANDOFF.md`'s
deferred item 3 (Postgres/MySQL backend, filed as #12) already flags:
`BEGIN IMMEDIATE` has no direct Postgres equivalent, and a real backend
would need its own per-row locking strategy to get genuine cross-window
parallelism instead of this store's serialize-everything behavior.

This was reasoned from the store's known design (documented in
`durable_control.rs`'s existing module doc comment and confirmed by the
single-connection pool + `BEGIN IMMEDIATE` code both sessions have now
read), not re-derived by a fresh experiment this session — no new claim is
made about actual parallelism, so none was needed.

## GitHub issues touched

- **#13** (new, Phase 4 proper): late data, corrections, and the
  coordinator/job API — `LatenessPolicy`/`CorrectionPlan`/
  `ReconciliationRecord`, scoped per the advisor's read of #10 to exclude
  compaction/retention/object-store, which #10 already covers. Cites
  `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 582-670 and cross-links
  #10, #11, #12 as explicit non-goals.
- **#14** (new, retry-loop coverage): `docs/TIMESERIES_PHASE_5_HANDOFF.md`
  deferred item 4 — `is_retryable`/`backoff`/`MAX_TX_ATTEMPTS` has zero test
  forcing it to fire under the shipped `BEGIN IMMEDIATE` config, needs a
  >5s lock-holder test (deliberately not added, to keep the suite fast),
  plus the silent-inert risk if `sqlx`'s `SqliteError::code()` format ever
  changes.
- **#7, #8, #9, #10, #11, #12**: untouched — no new findings against any of
  them this session; #13/#14 above cross-link the relevant ones rather than
  duplicating their scope.

## Deferred / not done this session

1. **Phase 4 proper** (late data, corrections, compaction —
   `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 582-670) has still not
   started; now tracked as #13 (minus compaction/retention, tracked in
   #10). This session closed one more of Phase 4's own test requirements
   (line ~637) against the existing claim/append/publish protocol, same
   pattern as the prior session's line 636 — neither is the phase itself.
2. **Multi-step CLI.** #11; not attempted.
3. **Postgres/MySQL backend.** #12; not attempted.
4. **Retry-loop coverage under the shipped `BEGIN IMMEDIATE` config.** Now
   tracked as #14; not attempted (needs a >5s test, a deliberate tradeoff
   the prior session made and this session re-confirmed rather than
   re-litigated).
5. **Real object store, DataFusion `TableProvider` exposure, state-blob
   checksum, compaction/retention** — unchanged, tracked as #10/#8/#9.
6. **The remaining Phase 4 test-list items beyond line ~637**: failure
   injection at every commit/publication stage, stale leases/expected
   revisions rejecting overwrites, and the late-event-rebuild-equals-clean-rebuild
   property test all still need Phase 4's actual types (`LatenessPolicy`,
   `CorrectionPlan`, coordinator) to exist first — tracked under #13, not
   independently attemptable as further protocol-level slices the way
   lines 636 and ~637 were.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, same
convention as `docs/TIMESERIES_PHASE_5_HANDOFF.md` adopted after
`docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a follow-up commit to fix a
self-referential hash). That commit contains:

- New: `docs/TIMESERIES_PHASE_6_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/tests/concurrency.rs` (new test +
  updated module doc comment, see above).
- Untouched: `crates/cubism-iceberg/src/durable_control.rs`,
  `crates/cubism-iceberg-spike/`, `crates/cubism-iceberg/src/config.rs`,
  `crates/cubism-iceberg/src/control.rs` — this session added a test against
  the existing store, it did not change the store itself.

Also present, deliberately uncommitted per prior-session convention: `.serena/`
(local tooling state), `examples/web_analytics_demo/events.csv` (generated
demo output).

## Tests (21 in `cubism-iceberg`: 8 unit + 6 Phase-3 integration + 3 durability integration + 4 concurrency integration, all passing)

New this session (`tests/concurrency.rs`):

- `disjoint_windows_can_commit_concurrently_without_conflicting` — see
  "What was actually verified" above for the precise (and limited) claim
  this test supports.

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 8 unit + 6 phase3, all passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 3 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # all passed, 1 ignored (unchanged from Phase 5)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-iceberg/tests/concurrency.rs`](../crates/cubism-iceberg/tests/concurrency.rs)
  (module doc comment updated; new test's own doc comment has the full
  "what this does and doesn't prove" writeup)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md) (Phase 4 spec, lines 582-670, line ~637 specifically)
- [`TIMESERIES_PHASE_5_HANDOFF.md`](TIMESERIES_PHASE_5_HANDOFF.md)
- GitHub issue [#13](https://github.com/jeromebanks/cubism-rs/issues/13) (Phase 4 proper, new this session)
- GitHub issue [#14](https://github.com/jeromebanks/cubism-rs/issues/14) (retry-loop coverage, new this session)
- GitHub issue [#12](https://github.com/jeromebanks/cubism-rs/issues/12) (Postgres/MySQL backend — cross-linked from this session's finding about global serialization)
