# Time-Series Phase 25 Handoff

Date: 2026-08-18

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 11** (`docs/TIMESERIES_ROADMAP.md`'s new "POC
Milestones" section) — minimal `/api/series` wiring in `crates/cubism-serve`.
The user asked, in conversation, to add a roadmap milestone for updating
`examples/web_analytics_demo` to demo real time-series data plus a
milestone for architecture/implementation documentation. That request was
first landed as its own roadmap-only commit (three new milestones: 11
`/api/series` wiring, 12 the demo, 13 the docs — the section preamble
records why this needed three, not one: the demo has nothing to query
without `/api/series`, and the docs milestone wants the demo as a worked
example). This session then picked up Milestone 11 itself, per
`.claude/skills/timeseries-slice/SKILL.md` step 1's "check the roadmap
first" rule — reachable immediately since its only dependency, Milestone
10b-3, was already `Done`.

Milestone 11 is not gated by step 8a's cross-model phase review: the "POC
Milestones" section is explicitly not one of
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md`'s phases (see the roadmap
section's own preamble), so it has no `## Phase N "done" condition` for a
slice to close. Ordinary per-slice advisor review (step 8) ran instead —
twice, both before any code was written: once to confirm Milestone 11 was
the right pick over `#20` (the highest-priority item on
`docs/TIMESERIES_PHASE_24_HANDOFF.md`'s deferred list, but design-decision-
shaped, not slice-shaped) and to check for a dependency-conflict risk
(closed: `cubism-serve` already resolves the same unified `arrow` 58.3.0
Phase 5's own investigation found, and already carried `cubism-datafusion`
as a dev-dependency with a `Cargo.toml` comment anticipating exactly this);
once more after finding that `crates/cubism-datafusion/src/range_query.rs:79-85`
had already rejected inventing a `WindowId` encoding as out of scope for a
milestone like this one, which reshaped the request wire format before any
handler code existed (see "What this session built" below).

Milestones 12 (the demo) and 13 (the architecture doc) remain `Not started`
— not attempted this session, deliberately: Milestone 11 is their shared
prerequisite (12 needs something to query; 13 wants 12 as a worked
example), and this series' own sizing convention is one bounded slice per
session, not three.

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

**The roadmap extension** (landed first, as its own commit, per this
series' precedent — `timeseries: extend roadmap to Phase 5, narrow #8
scope` landed before Milestone 7's own implementation the same way): a new
"POC Milestones — serving, demo, and documentation" section in
`docs/TIMESERIES_ROADMAP.md`, with Milestones 11-13 as described above. The
section explicitly states it does not supersede `#20` and is not 8a-gated,
both flagged by the advisor's first review pass as gaps in the initial
draft.

**Milestone 11 — `/api/series`.** `crates/cubism-serve/src/series.rs` (new):
a `SeriesState` (an open Iceberg catalog, `TemporalTable`, `PublicationStore`,
and `CubeSpec` for one cube) and a POST handler composing the Phase 5
pipeline against real I/O — `TemporalQuery::new` -> `ResolutionPlan::new`
-> `CoveragePlan::new` -> `AggregateReader::read_window` per published
window -> `SeriesResponse::new` — at the same narrowed scope Milestone
10b-3 already has: one `XUnit` selector, `AverageState` measures only.

The advisor's second review pass caught that the roadmap's own
first-draft text for this milestone (windows as a per-segment,
revision-carrying request list) would have re-derived a
segment→`WindowId` grouping the request itself would need to get right,
duplicating logic `ResolutionPlan` already computes server-side. The
shipped design instead takes a **flat** list of `(window_id, bucket_start)`
facts; the handler assigns each to whichever segment's `TimeRange::contains`
its `bucket_start`, and resolves each window's *current* revision itself
via `PublicationStore::current` rather than trusting a client-supplied one.
Net effect on the "untrusted client" caveat `series.rs`'s own module doc
comment describes (`SegmentCoverage::is_exact()` trusts the caller's
`published` list; `SeriesResponse` never re-verifies `batches` against it):
a request can misname a `window_id`, but cannot assert a stale or
fabricated revision for one that resolves, since revision is never
caller-supplied at all. See `docs/TIMESERIES_ROADMAP.md`'s Milestone 11
entry, "Deviation from this entry's own pre-written scope," for the full
account.

Supporting changes, all additive:
- `crates/cubism-datafusion/src/temporal_build.rs`: `measure_column_name`
  made `pub` (was crate-private) so `series.rs` resolves a request's
  measure name to its states-table column the same way the schema builder
  does, instead of re-deriving the `{name}_v{version}` naming rule.
- `crates/cubism-serve/src/api.rs`: `ApiError::bad_request` made
  `pub(crate)` so `series.rs` reports errors in the same `{"error": "..."}`
  envelope as the static-cube endpoints, rather than inventing a second
  shape.
- `crates/cubism-serve/src/lib.rs`: `router_with_series`/`serve_with_series`
  added alongside the existing `router`/`serve` — additive; the static-cube
  path and its doctest are unchanged.
- `crates/cubism-serve/Cargo.toml`: `cubism-datafusion` promoted from a
  dev-dependency (used only by `tests/api.rs`'s static-cube fixture) to a
  real one; `cubism-iceberg` and `iceberg = "=0.10.0"` (pinned to match
  `cubism-iceberg`'s own pin) added as real dependencies; `chrono` added as
  a dev-dependency for `tests/series.rs`'s fixture timestamps.
- `crates/cubism-cli/src/main.rs`: `serve`'s existing signature grew four
  new optional flags (`--spec`, `--warehouse`, `--catalog-db`,
  `--control-db`) — given `--spec`/`--warehouse` together, `serve` also
  opens a `SeriesState` and calls `serve_with_series` instead of `serve`;
  given neither, behavior is byte-for-byte unchanged from before this
  session (verified: the `(None, None)` match arm calls the same
  `cubism_serve::serve` the old one-argument function called directly).

**Advisor follow-up (applied in-session, before push, as this series'
convention prescribes — not a rewrite of the landing work above):**
`SeriesState::open` originally returned `Result<Self, String>`, off-
convention next to `CubeStore::from_path` -> `StoreError` and
`PublicationStore::sqlite` -> `cubism_iceberg`'s own typed `Result`, in a
crate that already depends on `thiserror`. Replaced with a new
`SeriesStateError` enum (`NoTemporalSpec`, `Schema` from
`datafusion::error::DataFusionError`, `Iceberg` from
`CubismIcebergError`, both via `#[from]`); `cubism-cli`'s one call site
gained an explicit `.map_err(|e| e.to_string())` to bridge back to its own
`Result<(), String>`. Also fixed: `crates/cubism-serve/Cargo.toml`'s
pre-existing `arrow = "58.3"` comment ("must track the arrow version
DataFusion 54 ships") read as contradicted by this session's own new
comment on `cubism-datafusion` just below it — resolved by stating plainly
that both the direct `arrow`/`parquet` deps (real, used by `store.rs`/`api.rs`
directly, confirmed not redundant with `cubism-datafusion`'s re-export) and
the version pin (keeping the "one unified arrow version" property
`cubism-iceberg`'s own top-level doc comment describes true rather than
accidental) are still load-bearing.

## What was actually verified

`crates/cubism-serve/tests/series.rs`'s single integration test,
`series_endpoint_answers_an_aligned_two_window_range_and_reports_an_unknown_window_as_missing`,
proves three things over a real HTTP listener (`series_router`, not a
direct function call) against a real durable backend (`CatalogConfig::Sqlite`
+ `PublicationStore::sqlite`, mirroring `cubism-iceberg/tests/durability.rs`'s
two-independent-handles pattern — fixture data appended/published through
one catalog/store handle, `SeriesState::open` given a second, so the test
cannot pass by accident of sharing in-process state):

1. An aligned `[2026-08-13T00:00:00Z, 2026-08-15T00:00:00Z)` range at `1d`
   resolution, with both backing windows real and published, returns one
   point: `is_exact: true`, `value` equal to the real `AverageState` merge
   of the two windows' accumulated values (`3.0, 5.0` and `10.0`, merged to
   `6.0` — not a hand-asserted constant; computed via `AverageState::merge`
   the same way the server does), both windows listed in `published` with
   their real revision.
2. A request naming a `window_id` that was never claimed, appended, or
   published (`PublicationStore::current` resolves it to `Ok(None)`, per
   `control.rs:315-318` — confirmed *not* an error path) comes back `200`
   with `is_exact: false`, `value: null`, and that window in `missing` —
   proving the untrusted-client exposure `series.rs`'s module doc comment
   flags does not manifest as a false-positive `is_exact: true` or an
   unhandled error.
3. The same unknown-window case with `exact: true` fails clearly with `400`
   and an error message naming the segment, via `CoveragePlan::new`'s own
   pre-existing `exact=true` rejection path — not a new check this
   milestone added, just the first place in the codebase that surfaces it
   over HTTP.

**What this does not prove:** `resolution: None` (auto-select) and
`gap_policy: "zero"` are exercised only by `range_query.rs`/`series_response.rs`'s
own unit tests, not this integration test. Nothing here proves the
TOCTOU gap `series.rs`'s own module doc comment documents (a concurrent
publish landing between `CoveragePlan`'s snapshot and the later
`read_window` call) — not solved this session, same open gap the
single-window CLI path already has. Nothing here exercises the
`cubism-cli serve --spec .. --warehouse ..` flag-parsing path directly
(only via `cargo build`/`cargo clippy`, both clean) — the integration test
calls `SeriesState::open`/`series_router` directly, not through the CLI
binary; a real end-to-end CLI smoke test is implicitly Milestone 12's job
once there's a real dataset to point it at.

## GitHub issues touched

None. Nothing this session's design decisions or caveats produced rose to
"new tracked issue" — both the TOCTOU gap and the "no RFC3339 parsing at
the request boundary, raw unix micros only" limitation are narrow,
already documented in `series.rs`'s own doc comments, and follow this
codebase's existing precedent of documenting a caveat inline rather than
filing an issue for every one (e.g. `SegmentCoverage::is_exact()`'s
own not-re-verified-against-`batches` caveat, `CoveragePlan`'s
caller-supplied-window-mapping decision — neither has its own issue
either). `#20` remains open and untouched, still the highest-priority item
on the deferred list below.

## Deferred / not done this session

1. **`#20`** (a delayed correction replay can silently undo an intentional
   rollback) — unchanged, still the most consequential open item; needs a
   design decision, not a mechanical fix, per
   `docs/TIMESERIES_PHASE_24_HANDOFF.md`.
2. **Milestone 12** (time-series web analytics demo) — the natural next
   slice: regenerate `examples/web_analytics_demo/events.csv` (or a
   variant) with late-arriving/out-of-order timestamps, build via
   `temporal-build`/`iceberg-build` in durable mode, and wire a query view
   against this session's `/api/series`. Owns picking the demo-local
   bucket→`WindowId` convention Milestone 11 deliberately left to its
   caller (roadmap entry's own note).
3. **Milestone 13** (architecture/implementation documentation) — blocked
   behind Milestone 12 by the roadmap's own "should be written after
   Milestone 12 lands" note, so it can use the demo as a worked example.
4. **A real CLI end-to-end smoke test for `cubism serve --spec ..
   --warehouse ..`** — not done this session (see "What this does not
   prove" above); natural to fold into Milestone 12's own build-script work
   rather than a standalone follow-up, since Milestone 12 needs exactly
   that invocation to work anyway.
5. **The TOCTOU gap between `CoveragePlan`'s publication snapshot and
   `read_window`'s own re-resolution of `current`** — documented in
   `series.rs`'s module doc comment, not solved; would need a locking
   primitive this crate does not have, same open gap as the
   concurrent-recovery item on `docs/TIMESERIES_PHASE_24_HANDOFF.md`'s own
   deferred list.
6. Every other item on `docs/TIMESERIES_PHASE_24_HANDOFF.md`'s deferred
   list (concurrent recovery of the same `run_id`, plan completion-criterion
   774/the SQL-table-function decision gated on `#8`, `#16`, `#10`, `#9`,
   `#14`, `#4`, `#3`) — unchanged, none touched this session.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hashes — this doc deliberately doesn't hardcode them, per
this series' established convention). Two commits this session: the
roadmap-only extension (Milestones 11-13 added), then this slice's
implementation.

- New: `crates/cubism-serve/src/series.rs`, `crates/cubism-serve/tests/series.rs`,
  `docs/TIMESERIES_PHASE_25_HANDOFF.md` (this file).
- Modified: `crates/cubism-serve/Cargo.toml`, `crates/cubism-serve/src/lib.rs`,
  `crates/cubism-serve/src/api.rs` (`ApiError::bad_request` visibility only),
  `crates/cubism-datafusion/src/temporal_build.rs` (`measure_column_name`
  visibility only), `crates/cubism-cli/src/main.rs` (`serve`'s signature and
  arg parsing, module doc comment, `USAGE`), `docs/TIMESERIES_ROADMAP.md`
  (new "POC Milestones" section; Milestone 11 marked `Done` with its
  deviation note), `docs/TIMESERIES_PHASE_24_HANDOFF.md` (`Superseded by`
  line added), `docs/handoff_latest.md` (symlink retarget), `Cargo.lock`
  (new dependency edges only).
- Untouched: `crates/cubism-iceberg/src`, `crates/cubism-core/src`,
  `crates/cubism-datafusion/src` beyond the one visibility change above
  (confirmed via `git status` after every verification pass this session).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (35 passed + 1 ignored in `cubism-iceberg`, unchanged; 93 passed across `cubism-core`'s suites, unchanged; 211 passed / 2 ignored in workspace, up from 210)

`cargo test -p cubism-iceberg` reports 35 passed, 1 ignored (6 suites) —
unchanged from Phase 24; this session touched no file under
`crates/cubism-iceberg/src`. `cargo test -p cubism-core` reports 83 passed
in its lib suite (93 total across all three of its suites: lib 83,
`phase1_properties.rs` 3, `properties.rs` 7) — unchanged from Phase 24;
this session touched no file under `crates/cubism-core/src`. `cargo test
--workspace --exclude cubism-py` reports 211 passed, 2 ignored (24 suites)
— up from Phase 24's 210/2/23 by exactly one new suite
(`crates/cubism-serve/tests/series.rs`) contributing its one new test.
`cubism-cli` still has no test harness of its own; verified via `cargo
build`/`cargo clippy` only, both clean.

## Verification performed

```text
cargo build -p cubism-serve                                                # clean
cargo clippy -p cubism-serve --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-serve                                                 # 3 passed (4 suites: lib 0, api.rs 1, series.rs 1, doctest 1)
cargo test -p cubism-iceberg                                               # 35 passed, 1 ignored (6 suites) — unchanged
cargo test -p cubism-iceberg --test concurrency                            # 4 passed, 1 ignored — unchanged
cargo test -p cubism-iceberg --test durability                             # 9 passed — unchanged
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings      # clean
cargo build -p cubism-cli                                                  # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings         # clean
cargo test -p cubism-core                                                  # 83 passed (lib), 93 total across its suites — unchanged
cargo clippy -p cubism-core --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                 # 211 passed, 2 ignored (24 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

Run in full once after the implementation was complete and the integration
test passing; `git status` checked after every pass. The full-workspace
`cargo test` total was cross-checked against a non-truncated capture
(`rtk proxy grep -n "test result:"` over the untruncated log, not a
`tail`-piped one — the skill's own step-4 note about a truncated capture
producing a wrong number before) before being cited above.

## Primary files

- [`../crates/cubism-serve/src/series.rs`](../crates/cubism-serve/src/series.rs)
  (new — `SeriesState`, the `/api/series` handler, `series_router`)
- [`../crates/cubism-serve/tests/series.rs`](../crates/cubism-serve/tests/series.rs)
  (new — this milestone's integration test)
- [`../crates/cubism-serve/src/lib.rs`](../crates/cubism-serve/src/lib.rs)
  (`router_with_series`, `serve_with_series`)
- [`../crates/cubism-serve/src/api.rs`](../crates/cubism-serve/src/api.rs)
  (`ApiError::bad_request` visibility)
- [`../crates/cubism-datafusion/src/temporal_build.rs`](../crates/cubism-datafusion/src/temporal_build.rs)
  (`measure_column_name` visibility)
- [`../crates/cubism-cli/src/main.rs`](../crates/cubism-cli/src/main.rs)
  (`serve`'s new optional series flags)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) ("POC
  Milestones" section; Milestone 11 marked `Done`)
- [`TIMESERIES_PHASE_24_HANDOFF.md`](TIMESERIES_PHASE_24_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#20`](https://github.com/jeromebanks/cubism-rs/issues/20)
  (untouched this session, still the top deferred item)
