# Time-Series Phase 17 Handoff

Date: 2026-08-15

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 7** (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5
Milestones" section) — the `RecordBatch`-from-`cubism-iceberg`-into-a-DF54-
`SessionContext` spike named as `docs/TIMESERIES_PHASE_16_HANDOFF.md`'s
deferred item 1 and confirmed by this session's advisor call as the
natural next slice. A new integration test in `crates/cubism-datafusion`
publishes a window via `cubism-iceberg`, reads it back with
`AggregateReader::read_window`, and hands the resulting `RecordBatch`es to
a DataFusion 54 `SessionContext` with no `iceberg-datafusion` dependency
anywhere in the call path. It compiles and passes. Milestone 7 flipped to
`Done` in the roadmap; commented on #8 with the empirical result,
narrowing what remains gated on it. One deviation from the roadmap's
original milestone text: `cubism-iceberg` landed as a `[dev-dependencies]`
entry rather than a plain `[dependencies]` one (advisor-flagged, see
"What this session built" below).

(Despite the filename, this doc documents a session slice, not "Phase 17"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving; the
plan's Phase 4 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself
fully done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_16_HANDOFF.md` and confirmed the branch in
sync with `origin` (`git rev-list --left-right --count` reported `0 0`),
then listed all open issues (`#1`-`#18`, unchanged from Phase 16). Phase
16's deferred item 1 named Milestone 7 as "the natural next slice" once
its own advisor review landed, and the roadmap's step-1 instructions say
to check the roadmap before falling back to an ad hoc scan — Milestone 7
was `Not started` with "Depends on: nothing new," so this session read
its full spec (`docs/TIMESERIES_ROADMAP.md:533-559`) and confirmed it with
the advisor before writing anything, per the skill's step 1.

The advisor confirmed Milestone 7 as the right slice and flagged nine
things worth acting on beyond the roadmap's own text:

1. **`[dev-dependencies]`, not `[dependencies]`.** The roadmap's "plain
   path dependency" wording would have pulled `iceberg`/`sqlx`/
   `iceberg-catalog-sql` into `cubism-datafusion`'s shipped dependency
   graph (and everything downstream of it) permanently, to serve a spike
   whose only consumer is `tests/iceberg_bridge.rs` — nothing in `src/`
   needs `cubism-iceberg` until Milestone 10. Landed as a dev-dependency
   instead (`crates/cubism-datafusion/Cargo.toml:18`); Milestone 10 can
   promote it in one line. Recorded as a deviation in the roadmap's
   Milestone 7 entry.
2. **No separate `arrow-array`/`arrow-schema` dev-deps** — build and
   consume batches through `datafusion::arrow::*` only, so the *compile
   succeeding* is the seam proof, not a coincidence of two independently
   declared `arrow` deps happening to resolve to the same version.
   `crates/cubism-datafusion/tests/iceberg_bridge.rs` never imports
   `arrow_array`/`arrow_schema` directly.
3. **Cargo/type friction check before writing the test:**
   `cubism-iceberg/src/lib.rs` does not re-export `iceberg::Catalog`
   (confirmed: its `pub use` list at lines 24-32 has no `Catalog`).
   Resolved by never naming the type explicitly — `open_catalog` returns
   `Arc<dyn Catalog>` and `.as_ref()` is called on it without importing
   the trait, so type inference carries the whole test without an
   `iceberg` dev-dependency at all.
4. **Publish before reading** — `read_window` rejects an
   appended-but-unpublished revision with `UnpublishedWindow`
   (`tests/phase3.rs:213-222` proves the same failure mode). The test
   calls `publications.publish("run-1", None)` before `read_window`.
5. **Verify the DF54 API rather than trust the roadmap's parenthetical.**
   Read `SessionContext::read_batches`'s actual source
   (`~/.cargo/registry/src/.../datafusion-54.0.0/src/execution/context/mod.rs:1785-1806`,
   confirmed via `Read` at that offset, not approximated) before writing
   against it — it takes `impl IntoIterator<Item = RecordBatch>` and
   returns `Result<DataFrame>`, matching the roadmap's `e.g.` usage
   exactly, so no `MemTable`/`register_table` fallback was needed.
6. **Sharper assertion than a bare `SELECT count(*)`.** `count(*)` over an
   in-memory table can be satisfied from row counts without decoding
   column data. The test asserts total row count *and* sums `count_v1`
   (plain `Int64`) across the collected batches via
   `.downcast_ref::<Int64Array>()` — the same manual-downcast pattern
   `crates/cubism-datafusion/src/build.rs`'s existing tests already use
   (e.g. `build.rs:224-226`), kept consistent with this crate's own
   convention rather than introducing a new SQL-string-based assertion
   style.
7. **Counts move, and not where Phase 16's numbers were.**
   `cubism-iceberg` stays 33 passed/1 ignored (unchanged — no source in
   that crate was touched); `cubism-datafusion` gains one suite
   (`iceberg_bridge`) and the workspace total moves 170→171 passed,
   22→23 suites. See "Tests" below for the fresh-run numbers.
8. **Bookkeeping beyond the skill's standard steps:** flip Milestone 7 to
   `Done` in the roadmap (done — `docs/TIMESERIES_ROADMAP.md:535-543`);
   comment on #8 with the empirical result (done —
   [comment](https://github.com/jeromebanks/cubism-rs/issues/8#issuecomment-5304417062)).
9. **A failing/non-compiling test would have been a legitimate landing
   outcome, not something to force green.** Not needed this session — the
   test compiled and passed on the first run
   (`cargo test -p cubism-datafusion --test iceberg_bridge`).

Primary files changed:

- **`crates/cubism-datafusion/Cargo.toml`** (modified): added
  `cubism-iceberg` and `tempfile` to `[dev-dependencies]` (lines 18-19),
  with a comment explaining the dev-dependency-not-dependency choice
  (lines 20-23).
- **`crates/cubism-datafusion/tests/iceberg_bridge.rs`** (new, 155 lines):
  the Milestone 7 spike test,
  `record_batch_from_read_window_round_trips_through_a_df54_session_context`
  (line 89), plus its doc comment (lines 1-20) stating what the test
  proves and does not prove — this series' standing convention for new
  tests, especially ones making a claim about a version/type seam that's
  easy to overstate.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 7's `**Status:**`
  flipped from `Not started` to `Done`, with the dev-dependency deviation
  recorded (lines 535-543).
- **`docs/TIMESERIES_PHASE_16_HANDOFF.md`** (modified): added
  `**Superseded by:**` line.

## What was actually verified

That a `Vec<RecordBatch>` read back from `cubism-iceberg`'s
`AggregateReader::read_window` — built and appended entirely through
`cubism-iceberg`'s own `arrow_array`/`arrow_schema` (DF53-era, `iceberg
0.10`'s dependency tree) — compiles and runs against a DataFusion 54
`SessionContext::read_batches` call with zero conversion code, using
exclusively `datafusion::arrow::*` types on the consuming side. Two rows
round-trip (`total_rows == 2`), and one column's actual values decode
correctly post-round-trip (`total_count_v1 == 8`, summed via
`Int64Array::downcast_ref`), not just the row count. That the full step-4
verification battery is clean with this change in place, re-run fresh this
session (see "Tests" below). That `cubism-iceberg` is reachable from
`cubism-datafusion`'s test binary only via `[dev-dependencies]`, not
`[dependencies]` (`cargo tree -p cubism-datafusion -e dev` shows the edge;
`cargo tree -p cubism-datafusion -e normal` does not). That `arrow`
58.3.0 is still unified across the DF53/DF54 split after this session's
`Cargo.toml` change (`cargo tree -i arrow --workspace`, re-run fresh this
session, not carried forward from Phase 16's pre-change resolution).

It does **not** prove: anything about `iceberg-datafusion`'s DF53
`TableProvider` / SQL-level predicate pushdown — that's still `#8`'s
open, narrower scope. It does not prove `xunit_id`'s `FixedSizeBinary(32)`
or `bucket_start`'s tz-annotated `Timestamp(Microsecond, "+00:00")`
decode correctly through DataFusion — only `count_v1` (plain `Int64`) is
asserted on beyond schema acceptance; the test's own doc comment says this
explicitly. It does not prove `read_window`'s single-window predicate can
be extended to a multi-window semijoin (`plan completion-criterion 774`,
still gated). It does not build any of Milestones 8-10's actual types —
Milestone 7 was a premise check, not `TemporalQuery`/`ResolutionPlan`/
`CoveragePlan` construction.

## GitHub issues touched

- [#8](https://github.com/jeromebanks/cubism-rs/issues/8) — commented
  with Milestone 7's empirical result: the direct-call functional path is
  now observed clear of the DF53/DF54 seam, not just reasoned from
  `cargo tree`. Left open — the SQL/pushdown half (criterion 774) is
  still real and still gated on this issue.
- No new issues filed. The advisor's slice-selection pass did not surface
  any deferred item newly sized to a bounded slice beyond Milestone 7
  itself.

## Deferred / not done this session

1. **Milestone 8 (`TemporalQuery` request shape + validation)** — the
   roadmap's next Phase 5 milestone (`docs/TIMESERIES_ROADMAP.md:560-577`).
   Its "Depends on: nothing new" and it's pure request-shape logic
   touching no DataFusion execution types, so it doesn't depend on
   Milestone 7 having landed — but Milestone 7 landing first keeps the
   roadmap's ordering intact and its premise (the seam works) now
   confirmed rather than assumed. Natural next slice.
2. **Milestone 9 (`ResolutionPlan`)** — depends on Milestone 8; unstarted.
3. **Milestone 10 (`CoveragePlan`/`SeriesResponse`)** — depends on
   Milestones 8 and 9, and is where `cubism-iceberg` gets promoted from a
   dev-dependency to a real one; unstarted.
4. **#17** (append-committed-but-not-recorded recovery) — unchanged; still
   needs a design decision before it can be sized into a bounded
   milestone. Not re-read this session (Phase 16 already re-confirmed its
   state; no reason to expect it changed).
5. **#16** (event-time window identification + recompute-equality proof)
   — unchanged; still needs a decision on which crate closes it. Phase
   16's note that it might relate to Milestone 7's new
   `cubism-datafusion`→`cubism-iceberg` dependency is now partially
   answered: that dependency is dev-only, not a `src/`-level one, so
   whatever #16 needs from `cubism-iceberg` is not yet reachable from
   `cubism-datafusion`'s production code — still a future slice's call,
   not resolved here.
6. **#10** (real object store + Iceberg maintenance) — unchanged.
7. **#11/#12** — unchanged; not touched this session.
8. **Plan completion-criterion 774** (storage pruning across many
   windows) — unchanged; still gated on #8's SQL/pushdown half, now
   narrower per this session's #8 comment.
9. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   not reached by Milestones 7-10.
10. **The `return Err(err)` branch's poisoning fix is unverified by a
    test** (carried forward unchanged from Phase 15/16 — still costs
    ~40s to exercise via `MAX_TX_ATTEMPTS` exhaustion; not re-investigated
    this session).

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention). That commit contains:

- New: `docs/TIMESERIES_PHASE_17_HANDOFF.md` (this file),
  `crates/cubism-datafusion/tests/iceberg_bridge.rs`.
- Modified: `crates/cubism-datafusion/Cargo.toml` (dev-dependencies),
  `docs/TIMESERIES_ROADMAP.md` (Milestone 7 status),
  `docs/TIMESERIES_PHASE_16_HANDOFF.md` (added `**Superseded by:**` line).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`,
  `crates/cubism-datafusion/src`, `crates/cubism-serve/src`,
  `crates/cubism-cli/` (built and clippy-checked, not modified).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`, unchanged; 171 passed / 2 ignored in workspace, up from 170; new `cubism-datafusion` suite `iceberg_bridge`: 1 passed)

All figures re-run fresh this session, not carried forward from Phase 16.
`cargo test -p cubism-iceberg` reports 33 passed, 1 ignored (6 suites) —
identical to Phase 16, expected since no source in that crate changed
this session. `cargo test -p cubism-datafusion` reports 31 passed (3
suites: the crate's existing unit/doc suites plus the new
`iceberg_bridge` integration suite, which alone reports 1 passed).
`cargo test --workspace --exclude cubism-py` reports 171 passed, 2
ignored (23 suites) — up from Phase 16's 170 passed/22 suites by exactly
the one new test.

## Verification performed

```text
cargo test -p cubism-datafusion --test iceberg_bridge                    # 1 passed
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-datafusion                                           # 31 passed (3 suites)
cargo test --workspace --exclude cubism-py                                # 171 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
cargo tree -i arrow --workspace                                           # single unified arrow 58.3.0, re-verified post Cargo.toml change
cargo tree -p cubism-iceberg                                              # confirms same arrow-array/arrow-schema 58.3.0
cargo tree -p cubism-datafusion -e dev                                    # shows cubism-iceberg as a dev-dependency edge
cargo tree -p cubism-datafusion -e normal                                 # confirms no cubism-iceberg edge in normal deps
```

## Primary files

- [`../crates/cubism-datafusion/tests/iceberg_bridge.rs`](../crates/cubism-datafusion/tests/iceberg_bridge.rs)
  (Milestone 7's spike test)
- [`../crates/cubism-datafusion/Cargo.toml`](../crates/cubism-datafusion/Cargo.toml)
  (`cubism-iceberg`/`tempfile` dev-dependencies, lines 15-23)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 7,
  lines 533-559; Milestone 8 is the natural next slice)
- [`TIMESERIES_PHASE_16_HANDOFF.md`](TIMESERIES_PHASE_16_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#8`](https://github.com/jeromebanks/cubism-rs/issues/8)
  (DataFusion 53/54 convergence — narrowed further this session with an
  empirical result, still open for the SQL/pushdown half)
