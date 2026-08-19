# Time-Series Architecture

Status: describes what is actually built on `feature/timeseries-phase-0a`
as of `docs/TIMESERIES_PHASE_28_HANDOFF.md`, the session that wrote this
doc (Milestone 13, `docs/TIMESERIES_ROADMAP.md`'s "POC Milestones"
section).

This doc has one job: let a reader unfamiliar with this session series
trace a single event from ingestion through to a served query answer,
without reading all 29+ phase handoffs to reconstruct the path. It is not
the spec — `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` stays authoritative
for intent, and is `Status: proposed plan, no production implementation
started` at its own header; this doc describes only what a file opened
this session, or a handoff already marked `Done`, actually shows. Where
the plan describes something wider than what exists (most measure kinds,
multi-selector queries, compaction, a scheduler), this doc says so and
points at the plan or the tracking issue instead of restating the intent
as if it were built.

This is also not `docs/architecture.md` — that doc covers the pre-existing
*static* cube pipeline (`cubism-cli run`, no time axis, `apiVersion: v1`).
Time is an axis around that same sparse `XUnit` lattice, not a
replacement for it; every stage below that touches lattice generation
reuses that pipeline's code unchanged, cited where it does.

Every claim below has a citation into either a source file (grep'd fresh
while writing this doc, not copied from an older citation) or a handoff
whose "What was actually verified" section already demonstrated it. If a
citation and the code disagree, trust the code and treat this doc as
stale for that line — this is the same posture the roadmap's own
milestone entries take toward the plan.

## The pipeline

The plan's target logical contract
(`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:19-25`):

```text
observed event
  -> allowed sparse XUnits
  -> event-time bucket
  -> versioned mergeable aggregate state
  -> immutable Iceberg window revision
  -> atomically published range-query visibility
```

Six stages, all built as of Milestone 10b-3 (Phase 5, narrowed — see
"What's narrowed" below). Milestones 11-12b add a seventh hop this
diagram doesn't cover on its own: **served answer**, a real HTTP client
reading stage 6 back out. That hop is not part of the plan's own
contract — it belongs to this roadmap's "POC Milestones" section, added
once the six-stage pipeline above already worked, specifically so it
could be *seen*.

| Stage | Crate | Primary module |
|---|---|---|
| 1-2. event -> sparse XUnits | `cubism-core` | `ypath.rs`, `lattice.rs` (shared with the static pipeline) |
| 3. event-time bucket | `cubism-core` | `temporal.rs` |
| 4. mergeable aggregate state | `cubism-core` + `cubism-datafusion` | `aggregate_state.rs`; `temporal_build.rs` (the build that produces it) |
| 5. immutable Iceberg window revision | `cubism-iceberg` | `writer.rs`, `control.rs`, `durable_control.rs`, `table.rs` |
| 6. published range-query visibility | `cubism-iceberg` + `cubism-datafusion` | `reader.rs`; `range_query.rs`, `series_merge.rs`, `series_response.rs` |
| 7. served answer (POC, not the plan's own contract) | `cubism-serve` + `cubism-cli` | `series.rs`; `main.rs`'s `serve`/`temporal-build`/`iceberg-build` subcommands |

## Worked example: one event, start to finish

The thread below is `examples/web_analytics_demo/query_temporal_demo.sh`
(Milestone 12b), reproducible by running that script. One concrete event
from a 6-event batch the demo's seeded generator deliberately holds back
from the first build:

- Event `evt_000261`: `event_name: signup_completed`, `plan: pro`,
  `revenue: 99`, event time inside window `2026-04-07` (UTC day bucket
  `[1775520000000000, 1775606400000000)` in Unix micros).
- Cube: `northstar_web_analytics_temporal`
  (`examples/web_analytics_demo/web_analytics_temporal.yaml`), one `geo`
  dimension (levels: `country`), one `avg_revenue` (`Avg`) measure over
  `revenue`, `baseResolution: 1d`.
- Built twice: revision 1 without this event (and 5 siblings — see
  below), revision 2 with it. Queried both times through a live
  `cubism serve` process, no restart between.
- Result: `/api/series` for `/G` (the global rollup) on window
  `2026-04-07` reports `avg_revenue` `0.0` at revision 1, then
  `1.6638655462184875` at revision 2 — `594 / 357`, six `pro`-plan
  conversions at `99` each over the window's 357 total event rows (most
  of which carry `revenue: 0`; `avg_revenue` averages every row's
  `revenue` field, not just conversions). `0.0` at revision 1 isn't a
  masked empty read — this seed's 6 late events happen to be *every*
  revenue-bearing row in the window, so revision 1 has a real, non-null
  average of zero. Milestone 11's own test shows what an actually-empty
  read looks like instead: `value: null`, `is_exact: false`
  (`crates/cubism-serve/tests/series.rs:248-255`).

Each stage below traces this same event.

## Stage 1-2: observed event -> allowed sparse XUnits

Unchanged from the static pipeline — time is an axis around this lattice,
not another dimension in it (`crates/cubism-core/src/temporal.rs:1-5`'s
own module doc comment states this explicitly). `evt_000261`'s row has
`country: DE`; row-local `XUnit`s are exploded from the spec's
dimension/level tree the same way a static build does — `XUnit`
(`crates/cubism-core/src/ypath.rs:74`), lattice generation
(`crates/cubism-core/src/lattice.rs:1-15`'s module doc comment), and the
global rollup marker `"/G"` (`crates/cubism-core/src/ypath.rs:13`, `XUnit::global()`
at line 79) that the worked example's query actually selects. A temporal
build reuses this verbatim — `crates/cubism-datafusion/src/temporal_build.rs`'s
own module doc comment (lines 1-16) states it generates "the *same*
row-local XUnits the static path would (same `level_columns`/
`cubism_xunit_keys` UDF, same lattice/rules code in `cubism-core`)".

Concretely, for this spec (one `geo`/`country` dimension, `includeGlobal:
true`): `evt_000261` lands in exactly two cells, the global rollup `/G`
and the per-country cell `/geo/country=DE` — confirmed empirically by
reading window `2026-04-07`'s own registry parquet
(`examples/web_analytics_demo/.temporal_build/registry_2026-04-07_r1.parquet`),
which contains six distinct canonical `XUnit`s total: the global rollup
plus one per-country cell for each of the demo's five countries (`AU`,
`CA`, `DE`, `GB`, `US`). This is the concrete fact Stage 6's selector
filtering (below) depends on: a query for `/G` alone must not silently
average across the other five per-country cells too.

## Stage 3: event-time bucket

`evt_000261`'s `timestamp` field is evaluated as the spec's
`temporal.eventTime` expression and assigned to one half-open UTC bucket.
The types: `TemporalSpec` (`crates/cubism-core/src/temporal.rs:596-610`,
`event_time`/`base_resolution`/`origin`/`allowed_lateness`/`rollups`
fields), `WindowId` (line 539, a validated non-empty string — this
demo's convention is one window per UTC day, `WindowId` = the day's start
date, e.g. `"2026-04-07"`; that convention is owned by the *caller*, not
this type — `cubism-core` never derives a `WindowId` from a time range
itself, see Stage 6), `FixedResolution` (line 97), `BucketOrigin` (line
213), `AllowedLateness` (line 377). The bucket-assignment arithmetic
itself lives in `FixedResolution::bucket` and is reused by the temporal
build, not re-derived in SQL
(`crates/cubism-datafusion/src/temporal_build.rs:6-10`).

**Not exercised in this worked example:** `AllowedLateness` is a real
type with a real value in the demo's own spec (`allowedLateness: 2h`),
but nothing in `temporal-build`/`iceberg-build`'s executable path reads
it — confirmed via `rtk proxy grep -rn "LatenessPolicy\|CorrectionCoordinator\|LatenessPolicy" crates/cubism-cli/src`
finding no call-site matches, only doc-comment prose. The 6 held-back
events are simply absent from one CSV and present in another; nothing
decided to admit them based on lateness. `LatenessPolicy`
(`crates/cubism-core/src/temporal.rs`, Milestone 1) and `CorrectionPlan`
(`crates/cubism-iceberg/src/correction.rs`, Milestone 3) both exist and
are tested, but nothing wires either into the CLI build path this demo
drives — that's the "Public API: submit/schedule a correction" bullet
[#13](https://github.com/jeromebanks/cubism-rs/issues/13) still lists as
open scope, not a separate untracked gap. (Not
[#20](https://github.com/jeromebanks/cubism-rs/issues/20) — that issue is
a specific replay bug in `CorrectionCoordinator::execute`'s own CAS call,
a different defect from "nothing calls `execute` outside its own tests"
in the first place.)

## Stage 4: versioned mergeable aggregate state

`evt_000261`'s `revenue: 99` accumulates into an `AverageState`
(`crates/cubism-core/src/aggregate_state.rs:182-209`): `(sum, count)`,
never a bare running mean — presented scalars are never treated as
authoritative merge state (the module's own doc comment, lines 1-5).
`AggregateState` is the shared trait (`accumulate`/`merge`/`present`/
`encode`/`decode`, lines 164-172) that every measure kind implements;
`AverageState::encode` writes a 3-byte magic (`AVG`,
`aggregate_state.rs:176`), a 1-byte format version (line 179), then the
raw `sum`/`count` bytes (lines 255-262) — a reader either recognizes the
magic+version or fails typed, never guesses. What this framing does
*not* have: a checksum of the payload bytes, so it detects an
unrecognized version but not corruption of an otherwise-recognized one —
tracked as [#9](https://github.com/jeromebanks/cubism-rs/issues/9), open
since Phase 3, not yet decided whether it's worth the byte-layout change.
The temporal build (`crates/cubism-datafusion/src/temporal_build.rs`)
runs this accumulation per `(bucket, xunit)` group via a DataFusion state
UDAF, emits Arrow batches sorted by `(bucket_start, xunit_id)`
(`temporal_state_schema`, line 139), and separately builds an
`xunit_id -> canonical bytes` registry via `SELECT DISTINCT` rather than
an inline accumulate-as-you-go map (module doc comment, lines 26-33) —
`build_temporal` at line 679 is the entry point
`cubism-cli`'s `temporal-build` subcommand calls.

**Narrowed:** only `AverageState` is exercised anywhere in this demo or
in Milestone 10b-2/10b-3's own read path; `SUM`/`COUNT`/`MIN`/`MAX`,
variance, quantile, and the sketch kinds all exist in `aggregate_state.rs`
but are not part of what `/api/series` (Stage 6-7) can answer today.

## Stage 5: immutable Iceberg window revision

Revision 1's states/registry batches for window `2026-04-07` are appended
via `AggregateWriter::append_window`
(`crates/cubism-iceberg/src/writer.rs:77`, `AppendWindow` request struct
at line 47) into a `TemporalTable`
(`crates/cubism-iceberg/src/table.rs:16`) — an ordinary Iceberg commit,
not yet visible to readers. Visibility is a separate, explicit act:
`PublicationStore::claim_run` (`crates/cubism-iceberg/src/control.rs:282`)
reserves the `run_id` (returns `ClaimResult`, line 88, so a retried claim
recognizes its own prior attempt rather than double-appending),
`record_append` (line 298) records the committed Iceberg snapshot id, and
`publish` (line 308) does the actual optimistic compare-and-swap against
a caller-supplied `expected_current: Option<WindowRevision>` — this is
the plan's "`WindowLease`/`ExpectedRevision`" mechanism, already built
before this roadmap's Milestone 2 existed to name it. A durable
(sqlite-backed) variant of the same store lives in
`crates/cubism-iceberg/src/durable_control.rs` (`SqliteStore`, imported
at `control.rs:6`) — the demo always runs in durable mode
(`--catalog-db`/`--control-db`), the same mode Milestone 12a's
`build_temporal_demo.sh` and Milestone 12b's `query_temporal_demo.sh`
both use.

Revision 2 is not a patch to revision 1 — it is a second, complete,
independent append-then-publish of the same window, from a fuller event
set. `WindowRevision` (`temporal.rs:566`) has no monotonicity check
baked into `publish` itself; repointing to an *older* run's `run_id`
against a fresh `expected_current` would repoint the control store
backward just as validly (`docs/TIMESERIES_PHASE_13_HANDOFF.md`,
confirmed empirically, not just read) — this is how this series' "Phase 4
done condition" rollback criterion is met, with zero new source code
(`docs/TIMESERIES_ROADMAP.md:571-580`).

## Stage 6: atomically published range-query visibility

A single-window read (verify/build-time, not a range query) goes through
`AggregateReader::read_window`
(`crates/cubism-iceberg/src/reader.rs:46`): resolve the current revision
from `publications`, scan the states table with a native Iceberg
`(window_id, revision)` equality predicate — not a DataFusion join
(module doc comment, `reader.rs:16-24`).

A *range* query — the shape the worked example's `/api/series` request
actually uses — goes through `cubism-datafusion`'s range-query module
instead: `TemporalQuery` (the request shape, `range_query.rs:169`,
validating `start < end` and that any explicit `resolution` is one the
spec supports — module doc comment lines 1-20), `ResolutionPlan`
(`range_query.rs:225-234`, selecting non-overlapping segments at one
resolution), `CoveragePlan` (`range_query.rs:392-410`, resolving each
segment's published/missing windows against the control store),
`merge_average_column` (`crates/cubism-datafusion/src/series_merge.rs:65`,
the actual value-materialization primitive — decode-then-`AggregateState::merge`
across every batch backing a segment), and `SeriesResponse`
(`crates/cubism-datafusion/src/series_response.rs:96-131`, wiring
`CoveragePlan` to a materialized value per segment).

`SeriesResponse`'s own module doc comment (`series_response.rs:37-49`) is
the single most load-bearing caveat in this whole pipeline, and it is
exactly what the worked example's `value: 0.0` at revision 1 depends on
being read correctly: **`is_exact` is copied straight from
`SegmentCoverage::is_exact()`, which reflects only the caller-supplied
`published`/`missing` lists — it is never re-verified against the actual
`batches` passed in.** A caller that mis-supplies `batches` for a segment
it itself reported as published gets a falsely-exact point with no data
behind it. This is why the worked example's README explicitly contrasts
`0.0` (a real merge over zero-revenue rows, `is_exact: true`, real data
behind it) against Milestone 11's unknown-window case (`value: null`,
`is_exact: false`,
`crates/cubism-serve/tests/series.rs:248-255`) — those are the two
different shapes an empty-looking answer can take in this API, and
conflating them is the exact failure mode this caveat warns about.

**Also narrowed here (Milestone 10b-3, fixing
[#19](https://github.com/jeromebanks/cubism-rs/issues/19)):**
`SeriesResponse::new` accepts exactly one `XUnit` selector — the
`selectors` slice must have length exactly 1 or the call is rejected with
`CubismError::Temporal`, not silently guessed at
(`series_response.rs:109-111`). The selector resolves to an
`XUnitContentId` via the same two calls (`CanonicalXUnit::from` +
`canonical_xunit_content_id`,
`crates/cubism-core/src/encoding.rs:62,324`) the build side's own UDF
uses, so a query-side selector and a build-side row resolve to
byte-identical ids for the same logical cell before any merging happens
(`series_response.rs:111-119`) — this is what makes the worked example's
`/G` selector actually filter to the global-rollup rows instead of
averaging across every `geo/country` cell in the window too.

**Not built:** `read_window`'s single-window predicate does not
generalize to a semijoin over every published window's manifest for a
range query (plan criterion 774, `docs/TIMESERIES_ROADMAP.md:1224-1233`)
— gated on [#8](https://github.com/jeromebanks/cubism-rs/issues/8)
(DataFusion 53/54 convergence for `iceberg-datafusion`'s
`TableProvider`). Per-point state/error metadata (plan line 716) is not
implemented — a decode/merge failure fails the whole `SeriesResponse::new`
call, not just the offending point
(`docs/TIMESERIES_ROADMAP.md:1234-1241`).

## Stage 7 (beyond the plan): served answer

This is where the worked example's two `curl`-equivalent captures
actually happen, and it's the hop the plan's own six-stage contract
doesn't include — added by this roadmap's "POC Milestones" section once
the engine above already worked, specifically to make it visible to
someone outside this session series
(`docs/TIMESERIES_ROADMAP.md:1284-1294`).

`SeriesState::open` (`crates/cubism-serve/src/series.rs:87`, struct at
line 76) holds one open Iceberg catalog handle, one `TemporalTable`, one
`PublicationStore`, and the cube's `CubeSpec` — one `SeriesState` per
cube. `series_router` (line 270) wires `POST /api/series` to it. This is
what `cubism-cli`'s `serve` subcommand
(`crates/cubism-cli/src/main.rs:407`) builds when given `--spec` and
`--warehouse` (plus `--catalog-db`/`--control-db` for durable mode) —
`temporal_build`/`iceberg_build` (lines 139, 249) are the same
subcommand's build-side counterparts, invoked by name at lines 513 and
577.

This is the actual `POST /api/series` request the worked example sends —
note the `windows` list's flat `(window_id, bucket_start)` shape, the
concrete form of Stage 3's "this convention is owned by the caller, not
`cubism-core`" claim:

```json
{"selector":"/G","measure":"avg_revenue","start":1775520000000000,"end":1775606400000000,"resolution":"1d","exact":false,"gap_policy":"missing","windows":[{"window_id":"2026-04-07","bucket_start":1775520000000000}]}
```

The two responses, same request, same running server, only the control
store's state between them changed (verbatim, re-captured this session
via `query_temporal_demo.sh`, not copied from an earlier session):

```json
{"cube":"northstar_web_analytics_temporal","measure":"avg_revenue","points":[{"bucket_end":1775606400000000,"bucket_start":1775520000000000,"is_exact":true,"missing":[],"published":[{"revision":1,"window_id":"2026-04-07"}],"value":0.0}],"source_resolution":"1d"}
```

```json
{"cube":"northstar_web_analytics_temporal","measure":"avg_revenue","points":[{"bucket_end":1775606400000000,"bucket_start":1775520000000000,"is_exact":true,"missing":[],"published":[{"revision":2,"window_id":"2026-04-07"}],"value":1.6638655462184875}],"source_resolution":"1d"}
```

**Confirmed empirically for the worked example, not assumed:** the
running `cubism serve` process answered the "after" query with the newly
published revision and the corrected value on the very next request,
with no restart in between — the control store and Iceberg table are
re-resolved per request, not cached at process-open time
(`docs/TIMESERIES_PHASE_27_HANDOFF.md`, "Confirmed empirically" note).

**A real finding from running the CLI, not something the in-process test
router would show:** `serve`'s positional `cube_path` argument is
required unconditionally — `CubeStore::from_path(cube_path)` is called
before `--spec`/`--warehouse` are even inspected
(`crates/cubism-cli/src/main.rs:415-417`) — even though nothing in
`/api/series`'s own handler ever reads the loaded `CubeStore`. Milestone
11's own integration test drives `series_router` directly and never
touches `cubism-cli`'s argument parsing at all, so it could not have
found this; Milestone 12b's script did, by running the real binary, and
worked around it with a throwaway one-row placeholder parquet
(`examples/web_analytics_demo/query_temporal_demo.sh`) rather than
depending on the static demo's own untracked cube file.

## What's narrowed, in one place

Everything above states its own narrowing inline; this table is a
lookup, not new information — follow the linked section or issue for the
real explanation.

| Gap | Where it's real | Tracking |
|---|---|---|
| `LatenessPolicy`/`CorrectionPlan` not wired into any CLI build path | Stage 3 above | [#13](https://github.com/jeromebanks/cubism-rs/issues/13) ("Public API" bullet) |
| Only `AverageState` reachable via `/api/series` | Stage 4, Stage 6 above | `docs/TIMESERIES_ROADMAP.md`'s Milestone 10b-2 "Deviations" |
| Range reads don't semijoin every published window's manifest | Stage 6 above (774) | [#8](https://github.com/jeromebanks/cubism-rs/issues/8) |
| Exactly one `XUnit` selector per query, no multi-selector shape | Stage 6 above | `docs/TIMESERIES_ROADMAP.md:1242-1247` (unresolved decision) |
| No per-point state/error metadata on a partial failure | Stage 6 above | `docs/TIMESERIES_ROADMAP.md:1234-1241` |
| No concurrent-request coverage for `/api/series` (read-after-plan non-atomicity) | Stage 6-7 | `crates/cubism-serve/src/series.rs`'s own module doc comment |
| Compaction, retention, object-store maintenance | not covered above at all | [#10](https://github.com/jeromebanks/cubism-rs/issues/10) |
| A delayed correction replay can silently undo an intentional rollback | Stage 5 above (rollback) | [#20](https://github.com/jeromebanks/cubism-rs/issues/20) |
| Event-time window identification / correction recompute is caller-owned, not derived | Stage 3, Stage 6 above | [#16](https://github.com/jeromebanks/cubism-rs/issues/16) |

## Where to go next

- `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` — the authoritative spec this
  doc synthesizes evidence against.
- `docs/TIMESERIES_ROADMAP.md` — the session-slice milestone list; its
  "Phase 4 done condition" (line 512) and "Phase 5 done condition" (line
  1179) sections are the two most detailed narrowing write-ups, more
  thorough than this doc's summary table above.
- `docs/phase-reviews/TIMESERIES_PHASE_4_REVIEW.md` and
  `TIMESERIES_PHASE_5_REVIEW.md` — cross-model review findings at each
  phase boundary.
- `docs/handoff_latest.md` — resolves to the current session's handoff;
  each handoff links backward to its predecessor via its own
  `**Superseded by**`/opening-paragraph chain, so this is the entry point
  into the full history rather than a 29-item list here.
- `examples/web_analytics_demo/README.md` — the worked example this doc
  traces, reproducible end to end.
