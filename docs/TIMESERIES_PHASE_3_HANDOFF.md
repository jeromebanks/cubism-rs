# Time-Series Phase 3 Handoff

Date: 2026-08-12

Branch: `feature/timeseries-phase-0a`

Status: **A complete, tested rollback-point slice of Phase 3 is implemented**
(`cargo test`/`clippy -D warnings` clean across `cubism-iceberg`, `cubism-cli`,
and the full workspace), **CLI-wired, and manually verified end-to-end
against real Parquet fixtures.** Not committed yet — see "Worktree state"
below.

## What this session built

Per `docs/TIMESERIES_IMPLEMENTATION_PLAN.md`'s Phase 3 (lines 469-580),
scoped down to what does not require a production infrastructure decision
(catalog vendor, object store, credentials) — those remain Phase 0A's own
unresolved gates (`docs/PHASE_0A_RESULTS.md`), not guessed at here.

A new crate, **`crates/cubism-iceberg`**, that makes Phase 2's `states`/
`registry` `DataFrame`s durable and queryable through Iceberg:

- **`error.rs`**: `CubismIcebergError` (thiserror), matching the
  `thiserror`-based-`CubismError`-per-crate convention `cubism-core`/
  `cubism-datafusion` already use — not `anyhow`, which only the
  non-production spike/bench crates use.
- **`config.rs`**: `CatalogConfig`, an enum with **one implemented
  variant**, `Memory { warehouse: PathBuf }` (in-memory Iceberg catalog +
  `LocalFsStorageFactory`). SQL/REST/Glue catalogs and real object stores
  are Phase 0A gates #1-#2 — deliberately not decided in this session.
- **`schema.rs`**: `states_iceberg_schema()` builds the Iceberg schema for
  the states table from Phase 2's own Arrow schema
  (`temporal_state_schema()`), prefixed with three identity columns
  (`window_id`, `revision`, `run_id` — see "Design decisions" below) with
  explicit, declared field IDs (not inferred). `xunit_registry_iceberg_schema()`
  is the fixed `(xunit_id, xunit_canonical)` shape. Day-partitions the
  states table on `bucket_start`, matching Phase 0B's selected
  `xunit_registry` physical layout and the Phase 0A spike's proven
  day-partition pruning; sorts on `(bucket_start, xunit_id)`, matching
  Phase 2's own output order.
- **`table.rs`**: `TemporalTable::create()` — creates (idempotently, via
  `table_exists` checks) both the `<cube_id>_states` and
  `<cube_id>_xunit_registry` tables under namespace `cube_id`, one
  states/registry table pair per cube (not one shared table across cubes,
  unlike the Phase 0A spike's single placeholder table).
- **`writer.rs`**: `AggregateWriter::append_window()` — the real,
  low-level Iceberg writer protocol, not a DataFusion `INSERT`:
  builds Parquet data files via `ParquetWriterBuilder` ->
  `RollingFileWriterBuilder` -> `DataFileWriterBuilder`, day-partitions the
  (already-sorted) states batches via a `ClusteredWriter` driven by
  `iceberg::transform::create_transform_function(&Transform::Day)` on the
  `bucket_start` column, commits via `Transaction::fast_append()`, and
  reads the **exact** committed snapshot ID off the `Table` the commit
  returns — resolving Phase 0A gate #4 (DataFusion `INSERT` only reports a
  row count) by never going through DataFusion `INSERT` at all. Rejects a
  duplicate data-file path within one append before it reaches
  `fast_append` (Phase 0A gate #3). The registry append commits first
  (content-addressed, idempotent-to-repeat); the states append — the run's
  real identity — commits second and is what `CommitResult::snapshot_id`
  reports, so a crash between the two leaves at most a harmless extra
  registry entry, never a states row without provenance.
- **`control.rs`**: `PublicationStore` — reuses the Phase 0A spike's
  proven `claim_run`/`record_append`/`publish` (CAS on
  `expected_current_revision`) protocol pattern
  (`crates/cubism-iceberg-spike/src/lib.rs`), keyed on `cubism-core`'s real
  `WindowId`/`WindowRevision` types (`crates/cubism-core/src/temporal.rs`)
  instead of the spike's bespoke composite key — `WindowRevision` had been
  defined since Phase 1 but was unused anywhere until now. **Not durable
  across process restarts** — an in-process `Mutex<HashMap>`. See "Design
  decisions" for why this matters more than "not yet production-grade."
- **`reader.rs`**: `AggregateReader::read_window()` — resolves the current
  revision from a `PublicationStore`, then scans the states table with a
  native Iceberg predicate (`Table::scan().with_filter(...)`) on
  `(window_id, revision)` and collects via `.to_arrow()`. No DataFusion,
  no join engine — each row already carries its own `window_id`/`revision`,
  so this is an equality filter over one window, not the plan's general
  "semijoin against every published window" (that's Phase 5, gated on
  DataFusion 53/54 convergence).
- **CLI**: `cubism-cli` gained `iceberg-build <spec.yaml> --states-input
  <parquet> --registry-input <parquet> --window-id <id> --revision <n>
  --run-id <id> --warehouse <path>` (hand-parsed `args()`, matching
  `temporal-build`'s existing pattern — no clap). One command, not the
  four separate `init`/`append`/`publish`/`verify` steps originally
  planned — see "Design decisions."

`crates/cubism-iceberg-spike` is untouched and unreferenced; it remains
the Phase 0A evidence trail, not a dependency of anything new.

## Design decisions worth knowing before extending this

- **`cubism-iceberg` links `iceberg` 0.10.0 directly and never links
  `iceberg-datafusion` or `datafusion` at all.** The rest of the workspace
  is on DataFusion 54; the released `iceberg-datafusion` 0.10 pulls
  DataFusion 53.1; their `SessionContext`/`DataFrame`/`TableProvider`/
  `ExecutionPlan`/`DataFusionError` types are not interchangeable. But
  `cargo tree` shows both sides resolve to the **same unified `arrow`
  58.3.0** — and `iceberg` 0.10's own low-level writer (`Transaction`/
  `FastAppendAction`/`DataFileWriter`) and native scan
  (`Table::scan().to_arrow()`) need nothing from DataFusion, only Arrow.
  So the crate boundary is Arrow `RecordBatch`/`Schema` only, and the
  DF53/54 seam never has to be crossed or managed — it simply doesn't
  arise. `TableProvider` exposure (the plan's stated Phase 3 "public API"
  bullet) stays deferred until DataFusion versions converge (Phase 0A gate
  #8); `AggregateReader` uses the native scan instead.
- **`FieldMatchMode::Name`, not the crate's default `Id`.** `iceberg`
  0.10's `ParquetWriterBuilder` matches incoming Arrow columns against the
  target Iceberg schema either by Arrow `PARQUET:field_id` metadata
  (`FieldMatchMode::Id`, the default) or by column name
  (`FieldMatchMode::Name`). Batches built by this crate's own
  `augment_states_batch` (and by any CLI/caller reading Phase 2's Parquet
  fixtures back into `RecordBatch`es) carry no Iceberg field-id metadata,
  so the default mode fails with `Field id 1 not found in struct array` —
  discovered empirically when the crate's own integration tests passed but
  the first real CLI smoke test against Parquet-round-tripped batches
  failed. Both writer paths (`write_states_partitioned`,
  `write_unpartitioned`) now call `.with_match_mode(FieldMatchMode::Name)`
  explicitly. Anyone adding a third write path needs the same call.
- **States table rows carry `window_id`/`revision`/`run_id` as real
  columns, prepended to Phase 2's own schema — `cube_id` does not.**
  `cube_id` is implicit in the table name (`<cube_id>_states`, one table
  pair per cube). The other three must be per-row: one states table
  accumulates every build of every window over time, and
  `AggregateReader` filters to "this window's current revision" by
  reading the `revision` column directly. `augment_states_batch` in
  `writer.rs` prepends these three constant-value columns to each Phase 2
  batch before writing; `schema.rs`'s field-ID constants
  (`WINDOW_ID_FIELD_ID`=1, `REVISION_FIELD_ID`=2, `RUN_ID_FIELD_ID`=3) are
  the single source of truth both sides use.
- **`CatalogConfig::Memory`'s namespace/table registry lives only in that
  process's RAM — this is a harder limit than "not yet durable," and it
  is why the CLI is one command, not four.** `iceberg` 0.10's
  `MemoryCatalog` keeps its namespace/table index in a
  `Mutex<NamespaceState>` that is never scanned from disk at open time;
  `LocalFsStorageFactory` only affects where data/metadata *files* land,
  not whether the catalog's own bookkeeping persists. A second CLI
  process pointed at the same `--warehouse` path gets a genuinely empty
  catalog, not the first process's tables — confirmed empirically: an
  initial `iceberg-init`-then-`iceberg-append`-then-`iceberg-verify`
  three-command CLI design (this session's original plan) built and
  passed every unit/integration test, then failed its first real smoke
  test with `NamespaceNotFound`, because `iceberg-append` had silently
  recreated its own disconnected copy of the tables in a fresh in-memory
  catalog. The CLI was collapsed to a single `iceberg-build` command that
  does create-if-needed/claim/append/record/publish/read-back in one
  process, and now smoke-tests correctly (see "Verification performed").
  A durable catalog (Phase 0A gate #1) is therefore not a "nice to have
  for production" — it is a hard prerequisite before *any* CLI or service
  can be split into separate init/append/publish/verify steps, and that
  is stronger than what the Phase 0A results doc originally implied.
- **`PublicationStore` is likewise in-process-only, independent of the
  catalog finding above.** Even with a durable catalog, today's
  `PublicationStore` (`Mutex<HashMap>`) would still lose all claim/publish
  state on process exit. `iceberg-build` therefore calls `publish(run_id,
  None)` — unconditional, not a real compare-and-swap against a prior
  revision, because there is no prior revision this process could ever
  see. It prints this limitation explicitly rather than silently
  pretending to enforce a CAS it cannot.
- **`cube_id` has exactly one source of truth: `TemporalTable.cube_id`.**
  An earlier version of `AppendWindow` also carried its own `cube_id`
  field (for "call-site clarity"), and `AggregateReader::read_window` took
  `cube_id` as a separate parameter from the `TemporalTable` it was also
  given — so a caller could pass a `TemporalTable` for one cube alongside
  a mismatched `cube_id` string, silently resolving the wrong cube's
  publication against the right cube's table. Neither this session's
  tests nor the CLI could catch it, since every call site always passed
  one consistent `cube_id`. Fixed by deleting the redundant fields;
  `writer.rs`/`reader.rs` now read `temporal_table.cube_id` directly, so
  there is nothing left to disagree.
- **No test crosses a process boundary; that is exactly why the
  `MemoryCatalog` finding above wasn't caught by the crate's own test
  suite.** Every `cubism-iceberg` integration test shares one
  `Arc<dyn Catalog>` per test fixture. The tests are still correct — they
  verify the in-process claim/append/publish/read protocol, which is all
  the crate itself claims to provide — but they cannot and do not exercise
  cross-process catalog visibility. The CLI smoke test is the only thing
  in this session that did.
- **No `checksum` in state-blob framing**, despite the plan doc's
  persistence section listing "magic/version/checksum." Confirmed
  unchanged from Phase 1/2:
  `cubism_core::aggregate_state::encode()` emits a 3-byte magic + 1-byte
  format version, no checksum
  (`crates/cubism-core/src/aggregate_state.rs`). Flagged, not changed —
  altering that framing is a `cubism-core` format/compatibility decision
  out of scope here.

## Tests (14 in `cubism-iceberg`: 8 unit + 6 integration, all passing)

- **Unit** (`control.rs`, sync, no Iceberg): deterministic retry returns
  the prior claim; claiming the same `run_id` with different inputs is
  rejected; publish requires the run to be appended first; a stale
  publish cannot replace a newer revision (and `current()` still reports
  the newer one); republishing the already-current revision is a
  harmless no-op.
- **Unit** (`writer.rs`, sync): `assert_unique_paths` passes unique paths
  and rejects a duplicate; `augment_states_batch` prepends the three
  identity columns in schema order.
- **Integration** (`tests/phase3.rs`, real Iceberg 0.10 + a local-fs
  `Memory` catalog, re-proving each Phase 0A finding against this crate's
  real production schema instead of the spike's placeholder one):
  - schema/field-ID/partition-transform/sort-order goldens for both
    tables;
  - `append_window` returns the **exact** committed snapshot ID (not a
    row count);
  - retrying the same `run_id` after a simulated crash (re-claim mid-way
    through the protocol) does not double-append — physical row count and
    the states table's snapshot ID are asserted unchanged;
  - an unpublished revision is invisible to `AggregateReader`;
  - publishing a new revision replaces visibility of the prior one (a
    stale `expected_current` is rejected first);
  - day-partition pruning still plans one file for a bounded scan
    (`plan_files()`, mirroring the spike's own pruning test).

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 14 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 150 passed, 1 ignored
```

Manual CLI smoke test against a real fixture (`temporal-build` writes
Parquet, `iceberg-build` reads it back):

```text
cubism temporal-build smoke_spec.yaml --input smoke_events.csv \
  --states-output states.parquet --registry-output registry.parquet
cubism iceberg-build smoke_spec.yaml \
  --states-input states.parquet --registry-input registry.parquet \
  --window-id smoke-window-1 --revision 1 --run-id run-1 \
  --warehouse ./warehouse
# -> snapshot <id>, 2 states file(s) (two day partitions), 1 registry
#    file(s), 4 row(s) written, 4 row(s) visible after publish
```

Confirms the Parquet -> Arrow round-trip preserves `bucket_start`'s
`+00:00` timezone (the exact trap `normalize_utc_timezones` exists to
guard against at a lower layer — here it round-trips correctly through
DataFusion's own Parquet writer/reader without needing that adapter).
Also exercised: missing required flags, `--revision 0` (rejected by
`WindowRevision::new`'s own validation), and re-running the same
`--run-id` against the same `--warehouse` path in a second process
(succeeds again rather than erroring or deduplicating — the documented,
expected behavior given no cross-process control-store state, not a bug).

## Deferred / not done this session

1. **Durable catalog and control store** (Phase 0A gates #1, and the
   corollary this session surfaced: a durable catalog is required before
   the CLI can be split into separate steps at all, not just before
   production). `CatalogConfig` stays single-variant; `PublicationStore`
   stays in-process.
2. **Real object store** (S3/GCS — Phase 0A gate #2). Local filesystem
   only, via `LocalFsStorageFactory`.
3. **`DataFusion TableProvider` exposure.** Blocked on DataFusion 53/54
   convergence (Phase 0A gate #8, unchanged).
4. **State-blob checksum.** `encode()`'s magic+version framing is
   unchanged from Phase 1/2; adding a checksum is a `cubism-core` format
   decision, not attempted here.
5. **Compaction, orphan cleanup, snapshot expiration, failure injection
   under a durable backend, Phase 0B's performance gate** — unchanged
   Phase 4/0B deferrals, same as Phase 2's own handoff.
6. **A `--registry-input`/`--states-input` window/CommitResult that
   deduplicates or reconciles a same-run-id retry across processes** —
   impossible without item 1; the CLI documents this rather than faking
   it.

## Worktree state

**Not committed.** Working tree has:

- New: `crates/cubism-iceberg/` (full crate: `Cargo.toml`, `src/{lib,error,
  config,schema,table,writer,control,reader}.rs`, `tests/phase3.rs`),
  `docs/TIMESERIES_PHASE_3_HANDOFF.md` (this file).
- Modified: `Cargo.toml` (new workspace member), `Cargo.lock`,
  `crates/cubism-cli/Cargo.toml` (new deps: `arrow-array`, `cubism-iceberg`,
  `futures`, `iceberg`, `parquet`), `crates/cubism-cli/src/main.rs`
  (`iceberg-build` subcommand).
- Untouched: `crates/cubism-iceberg-spike/` (Phase 0A evidence, not a
  dependency of anything new).

Also present, deliberately uncommitted per prior-session convention
(carried over from Phase 2, not touched this session): `.serena/` (local
tooling state), `examples/web_analytics_demo/events.csv` (generated demo
output).

## Primary files

- [`../crates/cubism-iceberg/src/writer.rs`](../crates/cubism-iceberg/src/writer.rs)
- [`../crates/cubism-iceberg/src/control.rs`](../crates/cubism-iceberg/src/control.rs)
- [`../crates/cubism-iceberg/src/reader.rs`](../crates/cubism-iceberg/src/reader.rs)
- [`../crates/cubism-iceberg/src/schema.rs`](../crates/cubism-iceberg/src/schema.rs)
- [`../crates/cubism-iceberg/tests/phase3.rs`](../crates/cubism-iceberg/tests/phase3.rs)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md) (Phase 3 spec, lines 469-580)
- [`PHASE_0A_RESULTS.md`](PHASE_0A_RESULTS.md) (the remaining gates this slice deliberately doesn't cross)
- [`TIMESERIES_PHASE_2_HANDOFF.md`](TIMESERIES_PHASE_2_HANDOFF.md)
