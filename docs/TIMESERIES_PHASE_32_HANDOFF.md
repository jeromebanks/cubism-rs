# Time-Series Phase 32 Handoff

Date: 2026-08-25

Branch: `feature/timeseries-phase-0a`

Status: **First demo slice landed.** `TIMESERIES_FINISH_PROMPT.md`'s
work-plan item 1 (the user-visible graph demo) is under way: this session
widened `/api/series` from avg-only to also admit the four scalar measure
kinds (count/sum/min/max) — roadmap Milestone 14, filed as the first of
the finish prompt's three planned demo slices ("API widening / chart /
docs-wiring"). Nothing is chartable yet: the demo spec still carries only
`avg_revenue`, the dashboard has no time-series chart, and
`build_temporal_demo.sh`'s wiring docs are untouched — those are the next
two slices. (Naming note, carried forward from every prior handoff in this
series: this file's number is a *session-slice* number, not a plan phase
number.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

- **`merge_scalar_column`**
  (`crates/cubism-datafusion/src/series_merge.rs:142`) — the scalar-kind
  counterpart to `merge_average_column`. Count/sum/min/max states columns
  are plain `Int64`/`Float64` scalars (`temporal_state_schema`'s declared
  storage), so there is no blob to decode: the merge *is* the aggregate's
  own fold (count sums in `i64`, converted once at presentation; sum/min/
  max fold in `f64`). "No data" is structural — a seen-flag, not an
  accumulator default — so min-of-nothing is `None`, not `0.0`, and a
  genuine zero count stays `Some(0.0)`. Rejects non-scalar kinds and
  kind/storage-type mismatches (`Count` against a Binary column, `Min`
  against Int64) rather than silently folding.
- **`SeriesResponse::new` takes the measure's `AggKind`**
  (`crates/cubism-datafusion/src/series_response.rs:152`) and dispatches
  blob-vs-scalar through a private `merged_value` helper
  (`crates/cubism-datafusion/src/series_response.rs:239`); gap policy
  applies to the no-data case only, for both paths. Blob-backed kinds
  beyond avg are now rejected with an explicit `CubismError::Temporal`
  instead of being unrepresentable at the type level.
- **The `/api/series` handler gate widened**
  (`crates/cubism-serve/src/series.rs:168`): count/sum/min/max
  measures pass where only avg did; everything else still gets a 400.
  The module doc's narrowing statement was extended consciously (single
  selector kept; measures widened), not silently rewritten.
- **Roadmap Milestone 14** (`docs/TIMESERIES_ROADMAP.md:1583`) added in
  the POC-milestones section per the 12a/12b split precedent — including
  why API widening went before a discovery endpoint: `PublicationStore`
  has no list-current-publications query on either backend, so that
  endpoint would be its own cross-crate slice, while the finish prompt's
  "(listing endpoint **or convention**)" branch is already satisfied by
  Milestone 12a's day-string convention.

Scope deliberately kept out, matching the advisor round: multi-selector /
multi-measure response shape (still the recorded unresolved roadmap
decision, still rejected at `SeriesResponse::new`'s exactly-one-selector
check), blob kinds beyond avg, discovery endpoint, dashboard chart, demo
spec/data changes, README/docs edits.

## What was actually verified

- The integration test
  (`crates/cubism-serve/tests/series.rs:311`,
  `series_endpoint_answers_a_count_measure_by_summing_the_published_windows_rows`)
  proves a count-only cube end to end over real durable Iceberg tables:
  two windows claimed/appended/published through one handle pair, served
  through an independently-opened second handle and live HTTP, answering
  `value: 7.0` (`3 + 4` across both windows' rows) with `is_exact: true`
  and both windows in `published`; plus a 400 for a measure name the spec
  doesn't have. It does **not** prove gap-policy substitution or selector
  filtering on the scalar path (unit-tested in `series_response.rs`, not
  here), and it does not exercise Sum/Min/Max over HTTP (same fold code
  path as Count modulo the combine step, which their unit tests cover).
- The scalar unit tests pin: cross-batch/row folding with null-skipping;
  min-of-mins / max-of-maxes; negative-sum handling; seen-flag `None`
  vs genuine-zero `Some(0.0)`; wrong-storage-type rejection; non-scalar-
  kind rejection; and selector filtering on the scalar path (a foreign
  cell's 1000-row count does not leak into `/G`'s 7).
- All pre-existing avg-path behavior is regression-covered unchanged
  (only mechanical call-site updates adding the `AggKind` argument).
- Not verified: anything about real web-analytics data or a rendered
  chart (later slices); counts larger than 2^53 in wire presentation
  (documented as a doc-comment caveat, no test).

## GitHub issues touched

None filed, none closed. The advisor confirmed the deferred scope is
already durably tracked: multi-selector/multi-measure shape lives in the
roadmap's recorded unresolved decisions (deliberately not issue-tracked),
and the discovery-endpoint idea gets filed only if/when the chart slice
demonstrates a client need beyond the documented day-string convention.

## Deferred / not done this session

1. **Dashboard line chart** (finish-prompt sub-part c): hand-rolled SVG
   fed by `/api/series` inside `crates/cubism-serve/assets/index.html`'s
   single-file offline page — no CDN library (the dashboard is
   `include_str!`-embedded). Next slice.
2. **Docs/wiring** (sub-part d): `examples/web_analytics_demo/README.md`,
   `docs/serving.md` (currently documents only the static-cube path — no
   `/api/series` mention), and whatever glue `build_temporal_demo.sh`'s
   output needs against `cubism serve` (note: `serve` still requires a
   static cube parquet positional; `query_temporal_demo.sh`'s throwaway
   placeholder precedent applies). Also the demo spec/dataset gaining a
   count measure (e.g. `page_views`) so a scalar series actually exists
   to chart.
3. **Discovery endpoint** (`GET /api/windows`-style): needs a new
   list-current-publications method across both `PublicationStore`
   backends + its own durability/concurrency legs. File an issue when
   scoped, per Milestone 14's entry.
4. Then per the finish prompt's order: durability tail sweep (#9, #14,
   guard hoisting, AwaitingAppend repro, SQLite refusal leg), #8 re-test
   spike, #16 engine link, Phase 6, #22 items.

## Worktree state

Committed to `feature/timeseries-phase-0a` (see `git log`; hashes
deliberately not hardcoded here).

- Modified: `crates/cubism-datafusion/src/series_merge.rs`,
  `crates/cubism-datafusion/src/series_response.rs`,
  `crates/cubism-datafusion/tests/iceberg_bridge.rs` (mechanical: new
  argument + import), `crates/cubism-serve/src/series.rs`,
  `crates/cubism-serve/tests/series.rs`, `docs/TIMESERIES_ROADMAP.md`
  (Milestone 14), `docs/TIMESERIES_PHASE_31_HANDOFF.md` (`Superseded by`
  line), `docs/handoff_latest.md` (symlink retarget).
- Untouched: every other source file; the demo assets under
  `examples/web_analytics_demo/`.

Deliberately uncommitted, prior-session convention: `.serena/`,
`examples/web_analytics_demo/events.csv`,
`.claude/skills/timeseries-slice/SKILL.md` pre-existing edits,
`docs/CODEX_TELEMETRY_ROADMAP.md`.

## Tests (44 passed + 1 ignored in cubism-iceberg; 227 passed / 2 ignored workspace)

cubism-iceberg total is unchanged (40 passed + 1 ignored — no iceberg
code touched). Workspace moved 216 -> 227 (+11): seven new
`series_merge.rs` unit tests, three new `series_response.rs` unit tests,
one new serve integration test.

## Verification performed

```text
cargo test -p cubism-iceberg                    # 40 passed, 1 ignored
cargo test -p cubism-iceberg --test concurrency # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability  # 10 passed, 0 ignored
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings  # clean
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings  # clean
cargo clippy -p cubism-serve --all-targets --no-deps -- -D warnings  # clean
cargo build -p cubism-cli                       # clean
cargo test --workspace --exclude cubism-py      # 227 passed, 2 ignored
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

rustfmt note: both `crates/cubism-serve` files report pre-existing diffs
under `rustfmt --edition 2024 --check` (issue #3), so all edits there are
hand-written in surrounding style; both `cubism-datafusion` files were
`--check`-clean and remain so.

## Primary files

- [`../../crates/cubism-datafusion/src/series_merge.rs`](../crates/cubism-datafusion/src/series_merge.rs)
  (`merge_scalar_column`)
- [`../../crates/cubism-datafusion/src/series_response.rs`](../crates/cubism-datafusion/src/series_response.rs)
  (`AggKind` dispatch)
- [`../../crates/cubism-serve/src/series.rs`](../crates/cubism-serve/src/series.rs)
  (handler gate + module doc)
- [`../../crates/cubism-serve/tests/series.rs`](../crates/cubism-serve/tests/series.rs)
  (count end-to-end test)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 14)
- [`TIMESERIES_PHASE_31_HANDOFF.md`](TIMESERIES_PHASE_31_HANDOFF.md)
  (prior handoff, superseded by this one)
- [`TIMESERIES_FINISH_PROMPT.md`](../TIMESERIES_FINISH_PROMPT.md) (the
  work plan this slice came from)
