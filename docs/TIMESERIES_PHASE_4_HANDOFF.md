# Time-Series Phase 4 Handoff

Date: 2026-08-12

Branch: `feature/timeseries-phase-0a`

Status: **Phase 4 itself (late data, corrections, concurrency, compaction —
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 582-670) has not started.**
This session cleared Phase 4's own hard prerequisite instead: durable
`cubism-iceberg` catalog and control-store backends
(`docs/TIMESERIES_PHASE_3_HANDOFF.md`'s design-decision note, tracked as
GitHub issue #7). `cargo test`/`clippy -D warnings` clean across
`cubism-iceberg`, `cubism-cli`, and the full workspace; manually verified
end-to-end with a **three-separate-OS-process** CLI smoke test. Committed
and pushed as `a820dec` — see "Worktree state" below.

## What this session built

Read `docs/TIMESERIES_PHASE_3_HANDOFF.md`, confirmed Phase 3's work was
already committed (`aee6a54`) and pushed, then filed/verified GitHub issues
#7-#10 for that handoff's six deferred items (already done by the prior
session; this one added item 6 as a corollary comment on #7 rather than a
near-duplicate issue, since it was explicitly "impossible without item 1").
`docs/TIMESERIES_PHASE_3_HANDOFF.md`'s "Not committed yet" line was stale
(the work *was* committed) — fixed.

Per the advisor's read of Phase 4's own test list ("two same-window writers
produce one published winner," "compare-and-swap the manifest against the
revision observed at planning time") — none of that is expressible against
a `Mutex<HashMap>` `PublicationStore` or a RAM-only `MemoryCatalog` — this
session treated issue #7 as the actual next slice, not Phase 4 proper:

- **`CatalogConfig::Sqlite`** (`crates/cubism-iceberg/src/config.rs`): a new
  variant using `iceberg-catalog-sql` 0.10.1 (the official SQL catalog from
  `apache/iceberg-rust`, matching this crate's pinned `iceberg = "=0.10.0"`)
  plus `LocalFsStorageFactory`, opened via `SqlCatalogBuilder` with
  `SqlBindStyle::QMark` and a `sqlite:<path>?mode=rwc` URI (SQLite's own
  `mode=rwc` query param handles create-if-missing; no separate
  `sqlx::Sqlite::create_database` call needed). `cargo tree` confirmed
  before writing any code that `iceberg-catalog-sql` and `sqlx` pull no
  `datafusion` and no second `arrow` — everything still resolves to the
  same unified `arrow` 58.3.0, so the crate's top-level "DF53/54 boundary
  never has to be crossed" invariant (Phase 3's central finding) still
  holds.
- **`PublicationStore::Sqlite`** (new module
  `crates/cubism-iceberg/src/durable_control.rs`, `SqliteStore`): a
  hand-rolled SQLite-backed implementation of the exact same claim/append/
  publish protocol as the in-process store — `iceberg-catalog-sql` only
  covers the Iceberg namespace/table registry, not this crate's own control
  store, so there was no off-the-shelf durable version of it. `PublicationStore`
  became an enum (`InMemory` / `Sqlite`) with every method now `async`;
  `.current()`/`.run_state()` changed from `Option<T>` to `Result<Option<T>>`
  so a real SQLite I/O failure can no longer be silently read as "not
  found." Every call site (`reader.rs`, `cli/src/main.rs`, `tests/phase3.rs`)
  updated to `.await`.
- **`crates/cubism-iceberg/tests/durability.rs`** (3 new integration
  tests): the specific thing Phase 3's own suite structurally could not
  catch — every fixture there shares one `Arc<dyn Catalog>` for the whole
  test. Every test here opens a **second, unrelated handle** (a fresh
  `Arc<dyn Catalog>` / a fresh `PublicationStore`) after dropping the
  first, pointed at the same on-disk paths — the same guarantee a second OS
  process needs. Covers: a fresh catalog handle sees the first's namespace
  and tables (including that `TemporalTable::create` is still idempotent
  from the fresh handle); an append committed by one handle is readable
  after a second handle reconciles the claim and publishes; a fresh
  `PublicationStore::Sqlite` handle performs a real CAS (`publish(run_id,
  Some(prior_revision))`) against a prior handle's publication, rejecting a
  stale `expected_current` it never observed in its own memory.
- **CLI**: `cubism-cli`'s `iceberg-build` gained optional `--catalog-db
  <path> --control-db <path>` flags (both required together, or neither).
  Omitted, behavior is byte-for-byte what Phase 3 shipped (non-durable
  `Memory` catalog + in-process store). Given both: uses the durable
  backends, and — now that a retried `run_id` can actually see its own
  prior state across processes — reconciles an `Existing(Appended)` /
  `Existing(Published)` claim by reusing the recorded snapshot ID instead
  of re-running `AggregateWriter::append_window`, and calls `publish` with
  the *actual* current revision read from the store (a real CAS) instead
  of the old hardcoded `None`. This directly resolves Phase 3's deferred
  item 6 ("a `CommitResult` that deduplicates a same-run-id retry across
  processes — impossible without item 1").

Also verified manually (not part of `cargo test`): `cubism temporal-build`
→ `cubism iceberg-build --catalog-db --control-db` run as **three separate
OS-process invocations** against one warehouse — first appends+publishes
window 1, second retries the identical `run_id`/window as a fresh process
(0 states/registry files rewritten, same snapshot ID, same visible rows),
third builds a second window from scratch against the same durable paths.
Also checked: omitting both flags still works (non-durable default,
unchanged message); passing only one of the two flags errors instead of
silently guessing.

## Design decisions worth knowing before extending this

- **`PublicationStore::Sqlite` transactions are plain deferred `BEGIN`, not
  `BEGIN IMMEDIATE`, and there is no busy-retry loop.** This is correct for
  sequential access and for the retry-after-restart scenario this session
  tested (open, act, drop, reopen, act) — SQLite's own transaction
  isolation gives correct read-then-write semantics there. It is **not**
  sufficient for two genuinely concurrent writers racing on the same
  window at the same instant; that needs `BEGIN IMMEDIATE` (or equivalent
  write-locking) plus `SQLITE_BUSY` retry, which is exactly Phase 4's own
  "two same-window writers produce one published winner" test
  (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` line 636) — deliberately not
  attempted here. Don't read the durability tests in
  `tests/durability.rs` as proof of concurrent-writer safety; they prove
  restart/reopen durability only.
- **`SqliteStore` uses a single-connection pool
  (`SqlitePoolOptions::max_connections(1)`)** so that one `PublicationStore`
  handle never races itself across async tasks within one process; a
  second handle/process still arbitrates purely through SQLite's own file
  locking (`busy_timeout(5s)` set on connect so lock contention waits
  instead of erroring immediately for the sequential-access case this
  session covers).
- **`iceberg-catalog-sql`'s own test suite binds SQLite with
  `SqlBindStyle::DollarNumeric`** despite its own doc comment saying
  `QMark` is for "SQLite/MySQL/MariaDB." This crate uses `QMark` per that
  doc comment, not per the upstream test's apparent inconsistency — worth
  a second look if a future `iceberg-catalog-sql` upgrade changes bind
  behavior and something here starts failing with a parameter-binding
  error.
- **`.current()`/`.run_state()` changed from `Option<T>` to
  `Result<Option<T>>`.** The old in-process store could never fail, so
  `Option` was fine; a real SQLite call can fail (I/O error, corruption),
  and collapsing that into `None` would make `AggregateReader::read_window`
  report a spurious `UnpublishedWindow` instead of surfacing the real
  error. `reader.rs` now does `.current(...).await?.ok_or_else(...)?` —
  the two failure modes stay distinguishable.
- **Row-status corruption in `control_runs`/`control_publications` becomes
  a new `CorruptControlStore(String)` error**, not a panic and not a silent
  fallback — e.g. a row whose `status = 'appended'` but
  `aggregate_snapshot_id IS NULL` (which the write paths never produce, but
  a hand-edited or externally-corrupted database could).
- **Deliberately did not re-split the CLI into `init`/`append`/`publish`/
  `verify`.** Per issue #7's own framing, that split is a natural
  follow-on now that durability exists, but it's a separate scope
  decision (multi-step invocations imply a different failure/retry
  contract per step) — this session's acceptance test is the three-process
  `iceberg-build` smoke test above, not a redesigned command surface.
- **`cargo tree` was run before writing any code**, per the advisor's
  explicit gate on this slice: confirmed `iceberg-catalog-sql` 0.10.1
  depends on `iceberg = "0.10.0"` (same pin), `async-trait`, `sqlx`,
  `strum` — no `datafusion`, and every `arrow-*` crate anywhere in
  `cubism-iceberg`'s dependency tree (including through `sqlx-sqlite`'s own
  transitive deps) resolves to `58.3.0`. Confirms Phase 3's "the crate
  boundary is Arrow `RecordBatch`/`Schema` only" claim still holds with
  these two new dependencies in the tree.

## Tests (17 in `cubism-iceberg`: 8 unit + 6 Phase-3 integration + 3 new durability integration, all passing; the 5 in-memory unit tests in `control.rs` converted from sync to `#[tokio::test]` for the new async API — no behavior change, same assertions)

New this session (`tests/durability.rs`):

- `sqlite_catalog_tables_are_visible_from_a_freshly_opened_handle` — create
  tables via handle A, drop A entirely, open handle B on the same
  `warehouse`/`catalog_db` paths, assert B sees both tables and that
  `TemporalTable::create` from B is still idempotent.
- `an_append_committed_by_one_handle_is_readable_after_publishing_from_a_second`
  — handle A claims+appends (does not publish), is dropped; handle B (fresh
  catalog *and* fresh `PublicationStore`) reconciles the claim as
  `Existing(Appended)` with the exact prior snapshot ID, reads the real
  current revision (`None`, correctly — not published yet), publishes, and
  reads the row back.
- `sqlite_publication_store_cas_survives_reopen_from_a_fresh_handle` — store
  A claims/appends/publishes revision 1, is dropped; store B sees revision
  1 via `.current()`, performs a real CAS `publish(run_id, Some(stale))`
  that is correctly rejected (`StaleRevision { expected: Some(99), actual:
  Some(1), .. }`), then a correct CAS that succeeds; store C (a third fresh
  handle) sees the result.

Phase 3's original 14 tests unchanged in substance, only in `.await` syntax
for the now-async `PublicationStore`.

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 17 passed
cargo test -p cubism-iceberg --test durability                            # 3 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 153 passed, 1 ignored
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

Manual three-process CLI smoke test (`temporal-build` writes Parquet once;
`iceberg-build` invoked three times as separate `cubism` process
invocations against one `--warehouse`/`--catalog-db`/`--control-db`):

```text
# process 1: fresh window, fresh run
cubism iceberg-build spec.yaml --states-input states.parquet --registry-input registry.parquet \
  --window-id smoke-window-1 --revision 1 --run-id run-1 \
  --warehouse ./warehouse --catalog-db ./catalog.sqlite --control-db ./control.sqlite
# -> snapshot 4340877339529048485, 1 states file(s), 1 registry file(s), 2 row(s) written, 2 row(s) visible

# process 2: identical run-id/window, separate OS process
cubism iceberg-build spec.yaml ... --window-id smoke-window-1 --revision 1 --run-id run-1 \
  --warehouse ./warehouse --catalog-db ./catalog.sqlite --control-db ./control.sqlite
# -> same snapshot 4340877339529048485, 0 states file(s), 0 registry file(s) (reconciled, not re-appended),
#    2 row(s) visible

# process 3: new window, same durable paths
cubism iceberg-build spec.yaml ... --window-id smoke-window-2 --revision 1 --run-id run-2 \
  --warehouse ./warehouse --catalog-db ./catalog.sqlite --control-db ./control.sqlite
# -> new snapshot 1053012025704309055, 1 states file(s), 1 registry file(s), 2 row(s) visible
```

Also exercised: non-durable default (flags omitted) still produces
byte-identical behavior to Phase 3's original CLI; passing exactly one of
`--catalog-db`/`--control-db` errors instead of silently choosing a
backend.

## GitHub issues touched

- **#7** (durable catalog + control store): posted a progress comment —
  the core finding is resolved (durable catalog + durable control store
  both exist and are tested across fresh handles), but two of its
  suggestions remain open: the multi-step CLI split, and real
  `BEGIN IMMEDIATE`/busy-retry arbitration for genuinely concurrent
  writers. Not closed.
- **#8, #9, #10**: untouched — DataFusion `TableProvider` exposure, the
  state-blob checksum gap, and real object store + maintenance are all
  still exactly as filed; nothing this session did changes any of them.
- Deferred item 6 from `docs/TIMESERIES_PHASE_3_HANDOFF.md` ("dedup/
  reconcile a same-run-id retry across processes") was added as a
  corollary comment on #7 rather than its own issue, then resolved by
  this session's CLI reconciliation logic — see the three-process smoke
  test above.

## Deferred / not done this session

1. **Phase 4 itself** (late data, corrections, concurrency, compaction —
   `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` lines 582-670) has not started.
   This session only cleared its prerequisite.
2. **Concurrent-writer arbitration** (`BEGIN IMMEDIATE` + `SQLITE_BUSY`
   retry in `SqliteStore`). Needed before Phase 4's "two same-window
   writers produce one published winner" test can be attempted for real;
   today's transactions are correct for sequential/retry access only.
3. **Multi-step CLI** (`iceberg-init`/`-append`/`-publish`/`-verify` as
   separate invocations). Newly unblocked by durability, but a distinct
   scope decision (per-step failure/retry contract) not made here.
4. **Real object store, DataFusion `TableProvider` exposure, state-blob
   checksum** — unchanged, tracked as #10/#8/#9 respectively.
5. **Postgres/MySQL catalog or control-store backends.** `iceberg-catalog-sql`
   supports them (it binds through `sqlx`'s `Any` driver), and
   `PublicationStore`'s `SqliteStore` schema would need only a bind-style
   change plus swapping `sqlx::sqlite::*` for `sqlx::any::*` — not
   attempted, since SQLite already satisfies every requirement this
   session's scope needed (a single-node durable local backend).

## Worktree state

**Committed and pushed** as `a820dec` on `feature/timeseries-phase-0a`.
That commit contains:

- New: `crates/cubism-iceberg/src/durable_control.rs`,
  `crates/cubism-iceberg/tests/durability.rs`,
  `docs/TIMESERIES_PHASE_4_HANDOFF.md` (this file).
- Modified: `Cargo.lock`, `crates/cubism-cli/src/main.rs` (`--catalog-db`/
  `--control-db` flags, retry reconciliation), `crates/cubism-iceberg/
  Cargo.toml` (new deps: `iceberg-catalog-sql`, `sqlx`), `crates/
  cubism-iceberg/src/{config,control,error,lib,reader}.rs`, `crates/
  cubism-iceberg/tests/phase3.rs` (`.await` on the now-async
  `PublicationStore`), `docs/TIMESERIES_PHASE_3_HANDOFF.md` (fixed the
  stale "Not committed yet" line).
- Untouched: `crates/cubism-iceberg-spike/` (Phase 0A evidence, still not a
  dependency of anything new).

Also present, deliberately uncommitted per prior-session convention: `.serena/`
(local tooling state), `examples/web_analytics_demo/events.csv` (generated
demo output).

## Primary files

- [`../crates/cubism-iceberg/src/durable_control.rs`](../crates/cubism-iceberg/src/durable_control.rs)
- [`../crates/cubism-iceberg/src/config.rs`](../crates/cubism-iceberg/src/config.rs)
- [`../crates/cubism-iceberg/src/control.rs`](../crates/cubism-iceberg/src/control.rs)
- [`../crates/cubism-iceberg/tests/durability.rs`](../crates/cubism-iceberg/tests/durability.rs)
- [`../crates/cubism-cli/src/main.rs`](../crates/cubism-cli/src/main.rs) (`iceberg-build`)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md) (Phase 4 spec, lines 582-670)
- [`TIMESERIES_PHASE_3_HANDOFF.md`](TIMESERIES_PHASE_3_HANDOFF.md)
- GitHub issue [#7](https://github.com/jeromebanks/cubism-rs/issues/7) (durable catalog + control store — this session's actual scope)
