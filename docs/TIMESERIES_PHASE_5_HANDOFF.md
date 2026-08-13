# Time-Series Phase 5 Handoff

Date: 2026-08-12

Branch: `feature/timeseries-phase-0a`

Status: **Phase 4 proper (late data, corrections, compaction —
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 582-670) has still not
started.** This session took one slice of it: the concurrent-writer
arbitration half of issue #7's two remaining suggestions (see
`docs/TIMESERIES_PHASE_4_HANDOFF.md`'s "Still open" comment on #7) — "two
same-window writers produce one published winner"
(`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` line 636). `cargo test`/`clippy -D
warnings` clean across `cubism-iceberg`, `cubism-cli`, and the full
workspace. Committed and pushed to `feature/timeseries-phase-0a` — see
"Worktree state" below for exactly what that commit contains.
(Despite the filename, this doc documents a session slice, not "Phase 5" of
the implementation plan — matching the established naming convention where
`TIMESERIES_PHASE_4_HANDOFF.md` documented issue #7's durability work, not
actual Phase 4.)

## What this session built

Read `docs/TIMESERIES_PHASE_4_HANDOFF.md`, confirmed its work was committed
(`a820dec`) and pushed, along with its same-day follow-up commit
`c2c92b3` (fixed a stale "Not committed yet" line the doc still had when
first committed — both confirmed already pushed). Per that handoff's
deferred-items list and issue #7's own progress comment, the two things
still open after durability landed were (a) concurrent-writer arbitration
and (b) the multi-step CLI split. This session took (a); (b) and one new
item (a Postgres/MySQL catalog/control-store backend) are filed as fresh
GitHub issues rather than attempted — see "GitHub issues touched" below.

- **`SqliteStore::with_immediate_tx`** (`crates/cubism-iceberg/src/durable_control.rs`):
  replaced `self.pool.begin()` (a plain deferred `BEGIN`, sqlx's only option
  via its typed `Transaction` API) with a hand-managed `BEGIN
  IMMEDIATE`/`COMMIT`/`ROLLBACK` sequence around a raw `PoolConnection`,
  wrapped in a bounded retry loop (`MAX_TX_ATTEMPTS = 8`, exponential
  backoff) that retries the whole sequence on `SQLITE_BUSY`/`SQLITE_LOCKED`
  and returns any other error (including a real protocol rejection like
  `RunConflict`/`StaleRevision`) immediately, never retried. `claim_run`,
  `record_append`, and `publish` all moved onto this helper; each now
  captures its `&str` arguments as owned `String`s before entering the
  retry closure, since sqlx's `for<'c> FnMut(&'c mut SqliteConnection) ->
  BoxFuture<'c, Result<T>>` closure shape cannot also borrow data with a
  shorter caller-bound lifetime (a Rust HRTB limitation, not a design
  choice — see the closure lifetime errors this produced before the fix).
- **`crates/cubism-iceberg/tests/concurrency.rs`** (3 new integration
  tests): covers Phase 4's line-636 requirement against
  `PublicationStore::Sqlite`. See "What was actually verified" below —
  this section is unusually load-bearing, because the first version of
  this test suite would have overclaimed coverage it didn't have.
- **Two GitHub issues filed** (multi-step CLI, Postgres/MySQL backend) and
  **one progress comment posted on #7** — see "GitHub issues touched."

## What was actually verified (read this before trusting the tests)

The first draft of `tests/concurrency.rs` asserted the right invariant but
its module doc comment claimed more than it had shown. Before writing this
handoff, three decisive experiments were run — each by temporarily
reverting `with_immediate_tx` to plain `BEGIN` and observing real behavior,
not by reasoning from SQLite documentation alone:

1. **The straightforward "two racing `publish` calls behind a barrier"
   test does not discriminate.** Reverted to plain `BEGIN` and run 300
   times, `two_same_window_writers_produce_exactly_one_published_winner`
   produced **zero failures**. Two `publish` calls against local SQLite
   complete in microseconds — far too fast to reliably land both
   transactions' reads before either commits. This test is a correctness
   assertion that holds under the shipped code; it is not demonstrated to
   catch a reintroduced deferred-`BEGIN` regression.
2. **A forced-contention test does discriminate on mechanism, not
   outcome.** `publish_waits_for_a_concurrently_held_write_lock_then_succeeds`
   opens a separate raw connection, holds a real `BEGIN IMMEDIATE` write
   lock for 200ms, and asserts `publish` still succeeds once it releases.
   With plain `BEGIN` (instrumented with `eprintln!` for this check, since
   reverted), the write inside `publish` hit an **immediate, repeating
   `SQLITE_BUSY`** ("database is locked") — a lock-*upgrade* conflict, which
   `busy_timeout` does not wait out — resolved only by 5 retries through
   `with_immediate_tx`'s own backoff loop. With real `BEGIN IMMEDIATE`, the
   identical scenario produced **zero retry-loop activity**: SQLite's own
   `busy_timeout` blocked and waited transparently on the initial lock
   request, succeeding on the first attempt. Both mechanisms end in success
   — the difference is which layer absorbs the wait, and only the fixed
   code makes that layer `busy_timeout` instead of a Rust retry loop that
   would otherwise need to fire on every contended request.
3. **No test reproduced the theoretical "lost update."** The concern that
   motivated this slice — two deferred transactions both reading
   not-yet-published state and both concluding their write is a valid CAS —
   remains a risk read off SQLite's documented rollback-journal locking
   model (no snapshot isolation to catch a stale read after the fact), not
   something observed in this codebase. Reproducing it black-box would need
   a test hook inside `with_immediate_tx` to pause between read and write,
   which doesn't exist.

**Net effect, stated precisely:** `BEGIN IMMEDIATE` serializes same-window
writers so exactly one publishes under any interleaving (true by
construction — SQLite's locking model guarantees it), the invariant is
asserted and passes under concurrent execution, and the one empirically
observed pre-fix failure mode was an opaque, repeating `SQLITE_BUSY` — not
silent corruption — now turned into either a transparent `busy_timeout`
wait or, as a backstop, a bounded Rust retry. Phase 4's line-636 requirement
is **addressed**, not proven by a regression test that would catch its own
reintroduction.

Full experiment details and the corrected module doc comments live in
`crates/cubism-iceberg/src/durable_control.rs` and
`crates/cubism-iceberg/tests/concurrency.rs` themselves — read those before
extending either file, since the reasoning is the documentation here.

## Design decisions worth knowing before extending this

- **The Rust-level retry loop (`is_retryable`/`backoff`/`MAX_TX_ATTEMPTS`)
  has no coverage under the shipped `BEGIN IMMEDIATE` configuration.** Every
  experiment that exercised it required reverting to plain `BEGIN` first
  (see above). As shipped, the loop's only exercised job is `COMMIT`'s
  `RESERVED`→`EXCLUSIVE` escalation blocking on a concurrent `SHARED`
  reader (`.current()`/`.run_state()` run outside any transaction) — not
  independently tested. If `sqlx`'s `SqliteError::code()` format ever
  changes, `is_retryable` would silently return `false` forever and this
  whole layer goes inert with nothing catching it. A test forcing this
  would need a holder lock held past `busy_timeout`'s 5s window (i.e., a
  >5s test) — not added here to keep the suite fast; worth adding if this
  path needs real confidence later.
- **Lost rollback-on-drop, a regression versus the code this replaced.**
  `sqlx::Transaction::drop` rolls back automatically if never committed.
  `with_immediate_tx` manages a raw `PoolConnection` with explicit
  `BEGIN IMMEDIATE`/`COMMIT`/`ROLLBACK` instead — required because sqlx
  0.8.1's typed `Transaction` API has no way to request `BEGIN IMMEDIATE` —
  so there is no automatic cleanup if a `claim_run`/`record_append`/
  `publish` future is dropped mid-operation instead of run to completion.
  Because `SqliteStore`'s pool has exactly one connection
  (`max_connections(1)`), a connection returned to the pool still
  mid-transaction poisons every later call on that handle with "cannot
  start a transaction within a transaction." Nothing in this codebase
  cancels these futures today (the CLI drives them sequentially to
  completion; every test `.await`s them) — a latent risk, not an active
  bug, but the first thing to check if a future session adds a timeout
  wrapper or a `tokio::select!` around one of these calls.
- **`claim_run`/`record_append`/`publish` now clone their string arguments
  once per retry attempt** (owned `String`s captured by the outer closure,
  cloned into each `Box::pin(async move { ... })` invocation). This is a
  direct consequence of the HRTB closure shape `with_immediate_tx` requires
  and is cheap for the short IDs this crate handles (`run_id`, `cube_id`,
  `window_id`) — not a performance concern at this scale, but a real cost
  if this pattern is reused for something with larger per-call payloads.
- **`docs/TIMESERIES_PHASE_4_HANDOFF.md`'s design-decisions section is left
  historically accurate, with a pointer added to this doc** rather than
  rewritten in place — same convention as that session correcting Phase
  3's stale "not committed yet" line: fix forward-pointing staleness with a
  note, don't erase the historical record of what a prior session actually
  shipped.

## Tests (20 in `cubism-iceberg`: 8 unit + 6 Phase-3 integration + 3 durability integration + 3 new concurrency integration, all passing)

New this session (`tests/concurrency.rs`):

- `two_same_window_writers_produce_exactly_one_published_winner` — two
  independent `PublicationStore::sqlite` handles claim/append different
  revisions for the same window, then race `publish(_, None)` behind a
  `tokio::sync::Barrier`; asserts exactly one `Ok(Publication)` and one
  `Err(StaleRevision { expected: None, actual: Some(winner) })`, that a
  third fresh handle's `.current()` matches the winner, and that the
  loser's `RunState` stays `Appended`, never `Published`. See "What was
  actually verified" above for what this test does and does not prove.
- `concurrent_publish_of_the_same_run_from_two_handles_is_idempotent` — the
  same run_id published concurrently from two handles; asserts both
  observe the identical `Publication`, not a race between two different
  outcomes.
- `publish_waits_for_a_concurrently_held_write_lock_then_succeeds` — an
  external raw connection holds a real `BEGIN IMMEDIATE` write lock for
  200ms; asserts `store.publish` still succeeds once it releases. The one
  test in this file with genuine forced contention.

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 20 passed
cargo test -p cubism-iceberg --test concurrency                           # 3 passed
cargo test -p cubism-iceberg --test durability                            # 3 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # all passed, 1 ignored (unchanged from Phase 4)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

Also run as a decisive (non-CI, throwaway) experiment: `with_immediate_tx`
temporarily reverted to plain `BEGIN`, `two_same_window_writers_...` run
300 times (0 failures), and
`publish_waits_for_a_concurrently_held_write_lock_then_succeeds` run once
with `eprintln!` instrumentation (5 retries observed, all through
`is_retryable`'s `SQLITE_BUSY` branch). Both experiments are described in
"What was actually verified" above and in the module doc comments of
`durable_control.rs`/`tests/concurrency.rs`; the instrumentation and
revert were not committed.

## GitHub issues touched

- **#7** (durable catalog + control store): posted a progress comment —
  concurrent-writer arbitration (one of its two remaining "still open"
  items) is now addressed per the precise claim in "What was actually
  verified" above. The other remaining item, the multi-step CLI split, is
  now its own issue (below) rather than staying as a sub-note on #7.
- **#11** (multi-step CLI, new this session): `iceberg-init`/`iceberg-append`/
  `iceberg-publish`/`iceberg-verify` as separate invocations — unblocked by
  durability, per-step failure/retry contract still undecided. Deferred
  item 3 from `docs/TIMESERIES_PHASE_4_HANDOFF.md`.
- **#12** (Postgres/MySQL catalog/control-store backend, new this session):
  both `CatalogConfig` and `PublicationStore::Sqlite` are wired through
  `sqlx` and `iceberg-catalog-sql` binds through `sqlx`'s `Any` driver, so
  this is a bind-style/driver swap plus a locking-model rewrite (SQLite's
  `BEGIN IMMEDIATE` has no direct Postgres equivalent), not a redesign.
  Deferred item 5 from `docs/TIMESERIES_PHASE_4_HANDOFF.md`.
- **#8, #9, #10**: untouched — DataFusion `TableProvider` exposure, the
  state-blob checksum gap, and real object store + maintenance are exactly
  as filed; nothing this session did changes any of them.

## Deferred / not done this session

1. **Phase 4 proper** (late data, corrections, compaction —
   `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 582-670) has still not
   started. This session closed one of Phase 4's own test requirements
   (line 636) as a standalone slice against the existing claim/append/
   publish protocol, not the phase itself — no `LatenessPolicy`,
   `CorrectionPlan`, `CompactionPlan`, or coordinator/job API exists yet.
2. **Multi-step CLI.** Filed as #11 this session; not attempted.
3. **Postgres/MySQL backend.** Filed as #12 this session; not attempted.
4. **Retry-loop coverage under the shipped `BEGIN IMMEDIATE` config** (see
   "Design decisions" above) — `is_retryable`/`backoff` are real code with
   zero test forcing them to fire in CI today. Would need a >5s
   lock-holder test; not added to keep the suite fast.
5. **Real object store, DataFusion `TableProvider` exposure, state-blob
   checksum** — unchanged, tracked as #10/#8/#9 respectively.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, to avoid
the self-referential-hash problem `docs/TIMESERIES_PHASE_4_HANDOFF.md`
needed a follow-up commit to fix). That commit contains:

- New: `crates/cubism-iceberg/tests/concurrency.rs`,
  `docs/TIMESERIES_PHASE_5_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/Cargo.toml` (`tokio` added as a direct
  dependency with `time`/`sync` features — previously only transitive via
  `sqlx`'s `runtime-tokio`), `crates/cubism-iceberg/src/durable_control.rs`
  (`BEGIN IMMEDIATE` + bounded retry, see above),
  `docs/TIMESERIES_PHASE_4_HANDOFF.md` (superseded-by pointer added to its
  design-decisions section, historical claims left intact).
- Untouched: `crates/cubism-iceberg-spike/`, `crates/cubism-iceberg/src/config.rs`,
  `crates/cubism-iceberg/src/control.rs` (the `PublicationStore` enum's
  public dispatch — unchanged, since `SqliteStore`'s method signatures
  didn't change, only their internals).

Also present, deliberately uncommitted per prior-session convention: `.serena/`
(local tooling state), `examples/web_analytics_demo/events.csv` (generated
demo output).

## Primary files

- [`../crates/cubism-iceberg/src/durable_control.rs`](../crates/cubism-iceberg/src/durable_control.rs)
  (module doc comment has the full experiment writeup)
- [`../crates/cubism-iceberg/tests/concurrency.rs`](../crates/cubism-iceberg/tests/concurrency.rs)
  (module doc comment has the full experiment writeup)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md) (Phase 4 spec, lines 582-670, line 636 specifically)
- [`TIMESERIES_PHASE_4_HANDOFF.md`](TIMESERIES_PHASE_4_HANDOFF.md)
- GitHub issue [#7](https://github.com/jeromebanks/cubism-rs/issues/7) (durable catalog + control store)
- GitHub issue [#11](https://github.com/jeromebanks/cubism-rs/issues/11) (multi-step CLI, new this session)
- GitHub issue [#12](https://github.com/jeromebanks/cubism-rs/issues/12) (Postgres/MySQL backend, new this session)
