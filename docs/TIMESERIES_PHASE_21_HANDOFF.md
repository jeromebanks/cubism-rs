# Time-Series Phase 21 Handoff

Date: 2026-08-16

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 10b-1** (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5
Milestones" section) — `merge_average_column`, the pure decode+merge
primitive that folds `AverageState` blobs read out of published windows'
states rows into one merged value. This is the "merging" half of Milestone
10's deferred "Milestone 10b" note; the advisor split that note in two
before any code was written (see "What this session built"): 10b-1 (this
session, the merge primitive alone) and 10b-2 (not yet scoped — a
`SeriesResponse` type wiring `CoveragePlan` to this primitive, plus
widening past `AverageState`). One new module
(`crates/cubism-datafusion/src/series_merge.rs`, six unit tests) plus one
new integration test against a real `cubism-iceberg` in-memory catalog.
Milestone 10b-1 added and flipped to `Done` in the roadmap; no new GitHub
issues filed. One real correctness bug caught by the integration test
itself (not by review): `AggregateReader::read_window`'s scan widens a
`Binary` column to `LargeBinary` on the way out of Iceberg, undocumented
until this session — fixed in the same commit, not a follow-up (see below).

(Despite the filename, this doc documents a session slice, not "Phase 21"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving; the
plan's Phase 4 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself fully
done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_20_HANDOFF.md` and confirmed the branch in sync
with `origin` (`git rev-list --left-right --count` reported `0 0`), then
listed all open issues (`#1`-`#18`, unchanged from Phase 20). Phase 20's
deferred item 1 named "Milestone 10b" (value materialization) as the
natural next slice, and the roadmap's own Milestone 10 entry pointed at the
same thing. Confirmed with the advisor before writing anything, per the
skill's step 1.

The advisor's guidance reshaped the slice before any code was written:

- **Phase-close check first, since it changes the whole session's shape.**
  The roadmap's "Phase 5 done condition" section pinned completion criteria
  772 and 775 specifically on "Milestone 10b" landing — so landing the
  original, unsplit "Milestone 10b" as written would have closed Phase 5
  and triggered the skill's step 8a cross-model phase review (a
  phase-sized Codex diff review plus a `docs/phase-reviews/` file) for the
  first time ever, on top of this slice's own work. The advisor's
  recommendation: don't take that on in one session — cut the work so it
  does not close Phase 5, and record the split in the roadmap. Taken:
  Phase 5 stays open; step 8a did not run this session (correctly — see
  "Deferred" below for what still gates Phase 5's close).
- **"Milestone 10b" as originally worded bundled three slices, not one:**
  (a) decode+merge `AggregateState` from `read_window`'s `RecordBatch`es;
  (b) a `SeriesResponse` type carrying value/presentation (plan line 708)
  and state/error metadata (plan line 716); (c) promoting `cubism-iceberg`
  from a dev-dependency to a normal `cubism-datafusion` dependency. Only
  (a) is slice-sized per this series' "one bounded next code slice"
  convention. Recommended cut, taken as-is: a pure/sync merge function
  scoped to **one** measure kind (`AverageState`), `cubism-iceberg` staying
  a dev-dependency, one new integration test publishing two windows,
  reading both, and asserting the merged value equals a merge computed the
  same way over the source rows.
- **The roadmap text being deviated from was itself wrong, caught before
  designing anything.** Milestone 10's own entry said the successor slice
  would merge "via `state_udaf.rs`" — checked against the actual source
  before writing code and found incorrect: `state_udaf.rs`
  (`crates/cubism-datafusion/src/state_udaf.rs`) is DataFusion `UDAF`
  machinery (`merge_batch` over `ArrayRef`s inside an execution plan), not
  a plain function callable outside a query plan. The actual reusable API
  is `cubism_core::aggregate_state::AggregateState`'s `decode`/`merge`
  (`crates/cubism-core/src/aggregate_state.rs:169,172`), which
  `state_udaf.rs` itself calls internally. `merge_average_column` uses that
  trait directly. Recorded as a correction on Milestone 10's own roadmap
  entry, not a silent fix.
- **Read the states-row layout before designing, per the advisor's
  instruction, not after:** `crates/cubism-iceberg/src/schema.rs` and
  `writer.rs`'s `append_window` confirmed rows are per-`(bucket_start,
  xunit_id)`, one row per bucket per dimension combination, with each
  measure as its own column — `AggKind::Avg` typed `Binary`
  (`crates/cubism-datafusion/src/temporal_build.rs:153`). This is what
  fixed the function's actual shape: fold every non-null blob in one named
  column across one or more caller-supplied batches, not a single blob per
  window.

Per the advisor's process note, before treating anything as newly
fileable, the open-issue list (`#1`-`#18`) was checked: nothing in it
covers value materialization; `#8` (DataFusion 53/54 `TableProvider`
convergence) is adjacent but distinct — read in full to confirm — since it
gates *SQL-level* range scans, not a direct-call merge over
caller-supplied batches. No overlap, no issue filed.

**A real bug the integration test caught, not code review:** the first
version of the new integration test failed at runtime with
`AggregateState("column 'avg_v1' is not a Binary array")`, even though
`temporal_build::temporal_state_schema` declares `AggKind::Avg` as
`DataType::Binary` and the test's own states batch was built with that
exact type. Root-caused by reading `iceberg` 0.10.0's own source directly
(`~/.cargo/registry/.../iceberg-0.10.0/src/arrow/schema.rs:690-692`):
`PrimitiveType::Binary` always converts to Arrow `DataType::LargeBinary` on
the way back out of a scan — a real, previously-undocumented type-widening
in `AggregateReader::read_window`'s output that this milestone's own doc
comment in `series_merge.rs` now records. Fixed in the same commit (not a
follow-up): `merge_average_column` matches on the column's actual runtime
`DataType` and handles both `Binary` and `LargeBinary`; a dedicated unit
test (`merge_average_column_handles_large_binary_and_mixed_batches`) proves
both are accepted, including within the same call. Not filed as a separate
issue — it doesn't block anything currently built (`CoveragePlan` never
touches this column) and is now handled, not just discovered; recorded in
the roadmap's Milestone 10b-1 entry for whichever slice builds
`SeriesResponse` (10b-2) next, since that slice will hit the same
widening.

**A second tooling finding, this session's own, not carried forward from
Phase 20's unidentified five-file incident:** running `rustfmt --edition
2024 --check crates/cubism-datafusion/src/lib.rs` (the crate root) reported
diffs in `build.rs` and `temporal_build.rs` — files this session never
touched. Root cause identified, not just observed: `lib.rs` is the crate
root with `pub mod build;`/`pub mod temporal_build;`/etc. declarations,
and `rustfmt`, given a crate-root file as input, walks and checks (or, if
run without `--check`, reformats) every file those declarations resolve
to — not just the one file named on the command line. This plausibly
explains Phase 20's own "five files this session never opened" incident
(`build.rs`, `state_udaf.rs`, `temporal_build.rs`, `udaf.rs`, `udf.rs` —
every one of them a module `lib.rs` declares): whatever ran `rustfmt` on
`lib.rs` there, explicitly or via some hook, would reformat exactly that
set. Not run against `lib.rs` this session at all (bare or `--check`) once
this was understood — the two-line diff this session made to `lib.rs`
(`pub mod series_merge;`, `pub use series_merge::merge_average_column;`)
was verified by eye instead (both lines short, alphabetically placed,
consistent with the file's existing style). Recorded against `#3` (the
existing rustfmt/toolchain-discrepancy issue), not filed separately.

Scope fence held per the advisor's confirmation: no `SeriesResponse` type,
no wiring from `CoveragePlan`'s `published` list to an actual value, no
measure kind besides `AverageState`, no `cubism-iceberg` dependency
promotion — all Milestone 10b-2's, not this slice's.

Primary files changed:

- **`crates/cubism-datafusion/src/series_merge.rs`** (new file, 207
  lines): `merge_average_column` (`:51-91`) plus its module doc comment
  (`:1-23`, including the `Binary`/`LargeBinary` widening note) and six
  unit tests (`:94-207`) covering multi-batch folding, null-skipping,
  empty input, the `Binary`/`LargeBinary`/mixed-batch shapes, a missing
  column, and a wrong-type column.
- **`crates/cubism-datafusion/src/lib.rs`** (modified, +2 lines): adds
  `pub mod series_merge;` and re-exports `merge_average_column`.
- **`crates/cubism-datafusion/src/range_query.rs`** (modified, doc comment
  only, `:104-112`): corrected the "Milestone 10b" module-doc reference to
  point at the now-split 10b-1 (landed)/10b-2 (not yet scoped) and at
  `series_merge::merge_average_column` by name.
- **`crates/cubism-datafusion/tests/iceberg_bridge.rs`** (modified): added
  `avg_states_schema`/`avg_states_batch` helpers (`:92-118`) and
  `merge_average_column_reads_and_merges_two_published_windows`
  (`:296-410`) — the real end-to-end wiring, publishing two windows with
  `AverageState`-encoded rows and asserting the round-tripped merge matches
  an in-memory merge of the same two states.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): added the "Milestone 10b-1"
  section (`:783-864`), flipped to `Done`; corrected Milestone 10's own
  "Deferred as Milestone 10b" bullet with a pointer to the split; updated
  the "Phase 5 done condition" walk's criteria 772/774/775 to reflect the
  merge primitive existing without yet being wired to `CoveragePlan`.
- **`docs/TIMESERIES_PHASE_20_HANDOFF.md`** (modified): added
  `**Superseded by:**` line.

No `Cargo.toml` change: `cubism-iceberg` stays a `cubism-datafusion`
dev-dependency (per the advisor's cut, same as Milestone 10) —
`merge_average_column` only needs `cubism_core`/`datafusion::arrow`,
already regular dependencies.

## What was actually verified

That `merge_average_column` correctly decodes and folds `AverageState`
blobs for six shapes, all pure unit tests in `series_merge.rs`: folding two
values across two separate batches produces the same result as merging
them directly (`merge_average_column_folds_multiple_batches_and_rows`); a
null blob in the column is skipped rather than causing a decode error
(`merge_average_column_skips_nulls`); an empty or all-null input returns a
zero state (`AverageState::new()`), not an error
(`merge_average_column_empty_input_is_zero_state_not_an_error`); a
`Binary` batch and a `LargeBinary` batch fold into the same merge, in the
same call (`merge_average_column_handles_large_binary_and_mixed_batches` —
this is the shape closest to what the integration test's real read-back
actually produces); a missing column name is rejected with
`CubismError::AggregateState`, not a panic
(`merge_average_column_rejects_missing_column`); and a column typed
neither `Binary` nor `LargeBinary` is rejected the same way
(`merge_average_column_rejects_non_binary_column`).

That the real end-to-end wiring works, via one `#[tokio::test]` against a
real `cubism-iceberg` in-memory catalog
(`merge_average_column_reads_and_merges_two_published_windows`,
`iceberg_bridge.rs:315`): two `AverageState` values are built via
`accumulate`, encoded, and written as the sole `avg_v1` row of two
separately claimed/appended/published windows (`AggregateWriter::append_window`
+ `PublicationStore::publish`, same pattern Milestone 7's round-trip test
established); both windows are read back via
`AggregateReader::read_window`; and `merge_average_column` over the
combined batches is asserted equal (`AverageState` derives `PartialEq`) to
`a.merge(&b)` computed directly from the two source states in memory. This
is what caught the `Binary`-vs-`LargeBinary` widening bug described above
— the unit tests alone, built before this integration test existed, would
not have caught it, since they construct `Binary` arrays by hand and never
exercise a real Iceberg scan's actual output type.

That the full step-4 verification battery — including `cargo clippy … -D
warnings` — is clean with this change in place, run fresh this session
(see "Tests"/"Verification performed" below), and re-run a second full
time after the `range_query.rs` doc-comment correction was added (to
confirm that small follow-up edit introduced no regression). `git status`
was checked after each verification pass; no out-of-band reformat of any
untouched file occurred this session (contrast Phase 20's five-file
incident, whose likely mechanism this session identifies — see "What this
session built" — without re-triggering it).

It does **not** prove: anything about `VarianceState`/`QuantileState`/the
sketch-backed kinds (`CountDistinct`/`TopK`/`ReservoirSample`/`Centroid`)
— `merge_average_column` only handles `AverageState`; those each need
their own merge wiring, not built here. It does not prove a
`CoveragePlan`-driven query can actually be answered end-to-end: nothing
in this session's diff calls `merge_average_column` from `CoveragePlan`'s
output, and no `SeriesResponse` type exists to carry the result — the
integration test builds its two windows directly, not through a
`ResolutionPlan`/`CoveragePlan`. It does not prove anything about storage
pruning across many windows (plan completion-criterion 774, unchanged,
still gated on `#8`). It does not identify whether the `lib.rs`
module-tree-walk rustfmt behavior is itself a bug in `rustfmt`, a
documented feature this crate's tooling doesn't account for, or something
else — only that it reproduces the Phase 20 symptom's file set exactly and
is now a known, avoidable trigger, not a mystery.

## GitHub issues touched

- No new issues filed. The `Binary`/`LargeBinary` widening is now handled
  in code (`merge_average_column`'s own match on runtime `DataType`), not
  an open gap needing a tracking issue; recorded in the roadmap's Milestone
  10b-1 entry instead, per this series' "record scope/behavior findings in
  the roadmap entry" precedent (Milestones 9-10 set it for their own scope
  cuts).
- No comments added to any other open issue. `#8` was read in full to
  confirm it does not already cover value materialization (it doesn't —
  it's about SQL-level `TableProvider` registration, a different seam) —
  same "read the full body before ruling out overlap" step the advisor's
  process note requires, no overlap found, no comment needed.

## Deferred / not done this session

1. **"Milestone 10b-2"** (not yet added to the roadmap as its own numbered
   milestone; recorded in Milestone 10b-1's own entry as the natural next
   slice). Build a `SeriesResponse` type carrying value/presentation (plan
   line 708) and state/error metadata (plan line 716); wire it to actually
   call `merge_average_column` for each `published` window a
   `CoveragePlan`'s `SegmentCoverage` reports; widen past `AverageState` to
   `VarianceState`/`QuantileState`/the sketch-backed kinds as needed; and
   promote `cubism-iceberg` from a dev-dependency to a normal
   `cubism-datafusion` dependency (confirmed, again, that no cycle blocks
   this — same finding Milestone 10 already made). Landing this is what the
   roadmap's "Phase 5 done condition" section now expects to close criteria
   772 and 775 — meaning it is also the slice that should trigger the
   skill's step 8a cross-model phase review; flag that explicitly at that
   slice's own step 1, don't let it surface as an afterthought.
2. **`lib.rs`-triggers-whole-module-tree-rustfmt finding** — plausibly
   explains Phase 20's five-file incident (see "What this session built"),
   but not proven as the *only* mechanism: this session did not attempt to
   reproduce Phase 20's incident by deliberately running bare `rustfmt` on
   `lib.rs` (that would have reformatted `build.rs`/`temporal_build.rs`
   for real, an unwanted side effect to trigger just to confirm a
   hypothesis). A future session investigating `#3` further could confirm
   by running bare `rustfmt --edition 2024 crates/cubism-datafusion/src/lib.rs`
   on a throwaway branch/worktree and diffing the result against Phase 20's
   own five-file diff.
3. **#17** (append-committed-but-not-recorded recovery) — unchanged; still
   needs a design decision before it can be sized into a bounded milestone.
   Not re-read this session (Phase 19/20 already re-confirmed its state; no
   reason to expect it changed).
4. **#16** (event-time window identification + recompute-equality proof) —
   unchanged; not touched by this session's work, which stays inside
   `cubism-datafusion`'s query-serving path, not the correction/rebuild
   path `#16` covers.
5. **#10** (real object store + Iceberg maintenance) — unchanged.
6. **#11/#12** — unchanged; not touched this session.
7. **Plan completion-criterion 774** (storage pruning across many windows)
   — unchanged; still gated on #8's SQL/pushdown half.
8. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   not reached by Milestones 8-10b-1.
9. **`#14`** (retry-loop/connection-poisoning gap) — unchanged, still open;
   not touched this session.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hashes — this doc deliberately doesn't hardcode them, per
this series' established convention):

- New: `docs/TIMESERIES_PHASE_21_HANDOFF.md` (this file),
  `crates/cubism-datafusion/src/series_merge.rs`.
- Modified: `crates/cubism-datafusion/src/lib.rs` (module declaration +
  re-export), `crates/cubism-datafusion/src/range_query.rs` (doc comment
  correction only), `crates/cubism-datafusion/tests/iceberg_bridge.rs` (new
  helpers + integration test), `docs/TIMESERIES_ROADMAP.md` (Milestone
  10b-1 added, Milestone 10's entry corrected, Phase 5 done-condition walk
  updated), `docs/TIMESERIES_PHASE_20_HANDOFF.md` (added
  `**Superseded by:**` line).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`,
  `crates/cubism-datafusion/src/{build,state_udaf,temporal_build,udaf,udf}.rs`
  (confirmed untouched via `git status` after every verification pass this
  session — no recurrence of Phase 20's out-of-band reformat, see "What
  this session built" for the identified mechanism this session avoided),
  `crates/cubism-serve/src`, `crates/cubism-cli/` (built and
  clippy-checked, not modified).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`, unchanged; 56 passed in `cubism-datafusion`, up from 49; 196 passed / 2 ignored in workspace, up from 189)

All figures re-run fresh this session, not carried forward from Phase 20,
and re-run a second full time after the `range_query.rs` doc-comment
correction. `cargo test -p cubism-iceberg` reports 33 passed, 1 ignored (6
suites) — identical to Phase 20, expected since no source in that crate
changed this session. `cargo test -p cubism-datafusion` reports 56 passed
(3 suites) — up from Phase 20's 49 by exactly the seven new tests (six
`series_merge` unit tests plus the one new integration test).
`cargo test --workspace --exclude cubism-py` reports 196 passed, 2 ignored
(23 suites) — up from Phase 20's 189 by exactly the same seven.

## Verification performed

```text
cargo test -p cubism-datafusion --lib series_merge                       # 6 passed
cargo test -p cubism-datafusion --test iceberg_bridge                    # 3 passed (2 prior + 1 new)
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-datafusion                                           # 56 passed (3 suites)
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings  # clean
cargo test --workspace --exclude cubism-py                                # 196 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

The `cubism-datafusion`/workspace test and clippy lines above were each run
twice this session: once after the `series_merge.rs`/`iceberg_bridge.rs`
change (including the `Binary`/`LargeBinary` fix), once more after the
`range_query.rs` doc-comment correction was added. Both runs produced
identical figures. Figures shown are from the final run.

## Primary files

- [`../crates/cubism-datafusion/src/series_merge.rs`](../crates/cubism-datafusion/src/series_merge.rs)
  (Milestone 10b-1's `merge_average_column`)
- [`../crates/cubism-datafusion/src/lib.rs`](../crates/cubism-datafusion/src/lib.rs)
  (module declaration + re-export)
- [`../crates/cubism-datafusion/src/range_query.rs`](../crates/cubism-datafusion/src/range_query.rs)
  (module doc comment correction, lines 104-112)
- [`../crates/cubism-datafusion/tests/iceberg_bridge.rs`](../crates/cubism-datafusion/tests/iceberg_bridge.rs)
  (new integration test, line 315)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone
  10b-1, lines 783-864; "Phase 5 'done' condition," lines 866+)
- [`TIMESERIES_PHASE_20_HANDOFF.md`](TIMESERIES_PHASE_20_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#3`](https://github.com/jeromebanks/cubism-rs/issues/3)
  (rustfmt/toolchain discrepancy — this session's `lib.rs`-triggers-
  module-tree-walk finding is a plausible root cause for Phase 20's own
  five-file incident, not yet confirmed by deliberate reproduction)
- GitHub issue [`#8`](https://github.com/jeromebanks/cubism-rs/issues/8)
  (read in full this session to confirm no overlap with value
  materialization)
