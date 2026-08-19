# Time-Series Phase 26 Handoff

Date: 2026-08-18

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 12a** (`docs/TIMESERIES_ROADMAP.md`'s "POC
Milestones" section) — a small, deterministic time-series variant of the
web analytics demo, built through the durable `temporal-build` +
`iceberg-build` CLI path, showing a real published revision bump for one
window. (Naming note, carried forward from every prior handoff in this
series: this file's number is a *session-slice* number, not a
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` phase number — this slice does
POC-milestone work, not plan-phase work.)

`docs/TIMESERIES_PHASE_25_HANDOFF.md`'s deferred list named "Milestone 12"
as the natural next slice. Per this skill's step 1, advisor review (before
any code) found Milestone 12 as originally written was **not** one bounded
slice: its own "Done when" has three independent clauses (late-arriving
data ingested; two published revisions of one `window_id`; a real
`/api/series` query reflecting the correction) spanning a Python
generator, a build script, and `site/` wiring — cross-language and
multi-deliverable, unlike every prior slice's "one integration test plus
supporting code" sizing (the same pattern that produced the 10b-1/10b-2/
10b-3 and 5b splits earlier in this series). The advisor recommended
splitting Milestone 12 into 12a (clauses a+b, this session) and 12b
(clause c, deferred) as a roadmap-only edit before implementing either,
and three cheap checks to find the split seam:

1. Does `web_analytics.yaml` already have a temporal section? No —
   confirmed via `crates/cubism-core/src/spec.rs:291-296`: `apiVersion: v1`
   rejects a `temporal` section outright, so a time-series variant needs a
   new `cubism/v2alpha1` spec file, not an edit to the existing one.
2. Does `temporal-build` require an `AverageState`-compatible measure
   (Milestone 11's narrowing)? Confirmed via the existing `AggKind::Avg`
   variant and `crates/cubism-serve/tests/series.rs`'s own fixture spec —
   a new spec needed exactly one `Avg` measure, not a KMV/distinct-count
   one.
3. Can the new spec coexist in one yaml with the existing one, or does it
   need its own file? Confirmed it needs its own file (check 1's answer),
   which also settled that `events.csv`/`web_analytics_cube.parquet`
   (the static demo's own generated outputs, left alone by this session)
   would not be touched.

Those checks came back exactly as the advisor predicted, which decided
the seam: 12a is the spec/dataset/build-script foundation with no Rust
changes at all; 12b (still `Not started`) is the `/api/series` query view
on top of it. This slice has **no `.rs` file changes** — see "What was
actually verified" below for what that means for the step-4 battery.

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

**The roadmap split** (landed as part of this session's own edits, not a
separate commit — unlike the Milestone 11 slice's precedent of a
roadmap-only first commit, this session's roadmap edit and 12a
implementation landed together since the split was decided before any
code was written and the two are small): Milestone 12's original entry in
`docs/TIMESERIES_ROADMAP.md` is marked "Superseded before implementation
by Milestone 12a and Milestone 12b," its original text left in place for
the historical record (additive, not rewritten, per this series'
convention), followed by new `### Milestone 12a` and `### Milestone 12b`
entries with their own Target/Test/Depends-on/Done-when fields. Milestone
13's "Depends on" note updated to name 12a/12b instead of the single
"Milestone 12."

**Milestone 12a — the time-series demo dataset and durable build.** Four
new files under `examples/web_analytics_demo/`, all additive (the static
demo's own `web_analytics.yaml`/`events.csv`/`generate_events.py`/
`web_analytics_cube.parquet` are untouched):

- `web_analytics_temporal.yaml` — a new `cubism/v2alpha1` spec: one `geo`
  dimension, one `avg_revenue` (`Avg`) measure, `temporal.eventTime:
  timestamp`, `baseResolution: 1d`. Narrowed to match Milestone 11's own
  scope (single `XUnit` selector, `AverageState` only) rather than
  mirroring the static demo's full dimension/measure set.
- `generate_temporal_events.py` — a small (220 users, 3 days, ~1,030
  events), seeded, deterministic generator producing two CSVs from the
  same underlying event set: `events_temporal.csv` (full) and
  `events_temporal_initial.csv` (the same stream with 6 revenue-bearing
  `signup_completed` events on the middle day held back) — the "initial"
  file is what the first build sees; the full file is what the corrected
  rebuild sees.
- `build_temporal_demo.sh` — drives `cubism temporal-build` +
  `cubism iceberg-build` (durable `--catalog-db`/`--control-db` mode) once
  per day for the two ordinary days, and twice for the middle day
  (2026-04-07): revision 1 from the initial stream, revision 2 from the
  full stream. Prints the resulting `control_runs`/`control_publications`
  rows for that window via `sqlite3` directly against the control store.
  All generated output (CSVs, the Iceberg warehouse, both sqlite
  databases) goes under `.temporal_build/`, added to `.gitignore` this
  session (mirrors the "generated, deliberately uncommitted" convention
  `events.csv` already established, but as a real gitignore entry rather
  than a manual staging discipline, since this variant produces a whole
  directory tree of sqlite/parquet files rather than one CSV).
- `README.md` — new "Time-series variant" section documenting the
  reproducible run, with the real captured `control_runs`/
  `control_publications` output pasted in verbatim (see "What was
  actually verified" below) and an explicit "what this does and does not
  prove" note.

**Scope narrowing found while implementing, not in the original Milestone
12 text:** the revision bump does **not** exercise `LatenessPolicy` or
`CorrectionPlan`. Confirmed via `rtk proxy grep -rn
"CorrectionPlan\|CorrectionCoordinator\|LatenessPolicy"
crates/cubism-cli/src` — the only matches are doc-comment prose about
`CorrectionCoordinator`'s crash-recovery behavior in an unrelated code
path (`iceberg-build`'s own retry logic), not a call into either type.
`iceberg-build` never reads `temporal.allowedLateness`. The revision bump
is produced by re-running the build over a fuller event set with a
manually-supplied `--revision 2` — a real, durable correction, but not a
demonstration of the admission-policy machinery. This is stated plainly
in both the roadmap's Milestone 12a entry and the demo's own README, per
this series' "don't let a passing check read as more coverage than it
has" convention, applied to a demo instead of a test.

**A throwaway pre-implementation check, not part of the shipped demo:**
before writing the real dataset/spec, this session ran a scratch
`temporal-build`/`iceberg-build` pair against a two-row fixture in
`/tmp` to confirm `claim_run`/`iceberg-build` actually accepts a second,
higher revision for a window already published at revision 1 (untested
ground — the Milestone 11 integration test only ever used `revision = 1`,
and `advisor()` flagged this as the one thing that could invalidate 12a's
whole shape before real implementation time was spent on it). It does:
revision 2 published cleanly with no rollback step required. That
scratch check is not part of the committed demo.

## What was actually verified

`build_temporal_demo.sh`, run once from a clean `.temporal_build/`
directory, produced this real output (captured, not paraphrased — also
pasted into `examples/web_analytics_demo/README.md`'s "Time-series
variant" section):

```text
== control_runs rows for window 2026-04-07 (revision bump) ==
run_id             window_id   revision  status
-----------------  ----------  --------  ---------
run-2026-04-07-r1  2026-04-07  1         published
run-2026-04-07-r2  2026-04-07  2         published

== control_publications current pointer for window 2026-04-07 ==
cube_id                           window_id   revision  run_id
--------------------------------  ----------  --------  -----------------
northstar_web_analytics_temporal  2026-04-07  2         run-2026-04-07-r2
```

This proves, over a real durable Sqlite catalog and control store (not an
in-memory fixture): the same `window_id` (`2026-04-07`) was published
twice, both runs recorded as `published` in `control_runs`, and the
control store's *current* pointer (`control_publications`, whose schema
is one row per `(cube_id, window_id)` — `crates/cubism-iceberg/src/durable_control.rs:159-166`)
moved from revision 1 to revision 2 once the held-back events were
included in the rebuild. Also proved along the way: `temporal-build`
correctly reads the `cubism/v2alpha1` spec and CSV input for three
different windows without error; `iceberg-build`'s durable mode
(`--catalog-db`/`--control-db`) works end to end for a spec/dataset
outside the workspace's own test fixtures.

**What this does not prove:** anything about `/api/series` actually
returning the corrected `avg_revenue` value — no HTTP request was made
this session; that is Milestone 12b's job, deferred below. Nothing about
`LatenessPolicy`/`CorrectionPlan` (see the scope-narrowing note above).
Nothing about the numeric `avg_revenue` value itself changing in a
sensible way — this session verified the *revision bump mechanism*, not
the resulting value; Milestone 12b's query view is what will surface the
actual before/after numbers.

Since this slice touches no `.rs` file, the standard test-count claims are
**unchanged from Phase 25**: `cargo test -p cubism-iceberg` reports 35
passed, 1 ignored (6 suites); `cargo test -p cubism-core` reports 83
passed in its lib suite (93 total across all three suites); `cargo test
--workspace --exclude cubism-py` reports 211 passed, 2 ignored (24
suites) — identical to Phase 25's counts, because nothing under any
crate's `src/` or `tests/` changed. `rustfmt` was not run this session —
per the skill's own step-2 note, its `--check` gate only matters for
files being edited, and no `.rs` file was.

## GitHub issues touched

None. This slice's own scope-narrowing (no `LatenessPolicy`/
`CorrectionPlan` exercised) is documented inline in the roadmap entry and
the demo's README, following this series' existing precedent of
documenting a caveat inline rather than filing an issue for every one
(Phase 25's handoff lists several prior examples). `#20` remains open and
untouched, still the highest-priority item on the deferred list below.

## Deferred / not done this session

1. **`#20`** (a delayed correction replay can silently undo an intentional
   rollback) — unchanged, still the most consequential open item; needs a
   design decision, not a mechanical fix.
2. **Milestone 12b** (query view onto Milestone 12a's demo, hitting
   `/api/series`) — the natural next slice: start `cubism serve --spec
   web_analytics_temporal.yaml --warehouse .temporal_build/warehouse
   --catalog-db .temporal_build/catalog.sqlite --control-db
   .temporal_build/control.sqlite` against this session's build output and
   capture a real request/response pair showing `avg_revenue` differing
   before/after the revision-2 correction, both `is_exact: true`. See the
   roadmap's Milestone 12b entry for the request shape.
3. **Milestone 13** (architecture/implementation documentation) — blocked
   behind both 12a (done) and 12b, per the roadmap's own "should be
   written after Milestone 12 lands" note (now naming 12a/12b).
4. **A real CLI end-to-end smoke test for `cubism serve --spec ..
   --warehouse ..`** — still not done (carried over from Phase 25's own
   deferred list); natural to fold into Milestone 12b, since 12b needs
   exactly that invocation to work anyway.
5. **The TOCTOU gap** between `CoveragePlan`'s publication snapshot and
   `read_window`'s own re-resolution of `current` — unchanged, documented
   in `series.rs`'s module doc comment, not touched this session (no
   `.rs` file changed).
6. Every other item on `docs/TIMESERIES_PHASE_25_HANDOFF.md`'s deferred
   list (concurrent recovery of the same `run_id`, plan completion-criterion
   774/the SQL-table-function decision gated on `#8`, `#16`, `#10`, `#9`,
   `#14`, `#4`, `#3`) — unchanged, none touched this session.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention).

- New: `examples/web_analytics_demo/web_analytics_temporal.yaml`,
  `examples/web_analytics_demo/generate_temporal_events.py`,
  `examples/web_analytics_demo/build_temporal_demo.sh`,
  `docs/TIMESERIES_PHASE_26_HANDOFF.md` (this file).
- Modified: `docs/TIMESERIES_ROADMAP.md` (Milestone 12 marked superseded;
  Milestones 12a/12b added; Milestone 13's "Depends on" updated),
  `examples/web_analytics_demo/README.md` (new "Time-series variant"
  section), `.gitignore` (new `examples/web_analytics_demo/.temporal_build/`
  entry), `docs/TIMESERIES_PHASE_25_HANDOFF.md` (`Superseded by` line
  added), `docs/handoff_latest.md` (symlink retarget).
- Untouched: every crate under `crates/` — this slice changed no `.rs`
  file, confirmed via `git status` after every step this session.

Also present, deliberately uncommitted: `.serena/` (local tooling state,
prior-session convention); `examples/web_analytics_demo/events.csv` (the
*static* demo's own generated output, unrelated to this session, prior-
session convention); `examples/web_analytics_demo/.temporal_build/` (this
session's generated demo output — the Iceberg warehouse, both sqlite
databases, and the temporal event CSVs — now gitignored rather than just
manually excluded, since it's a whole directory tree rather than one
file).

## Tests (35 passed + 1 ignored in `cubism-iceberg`, unchanged; 93 passed across `cubism-core`'s suites, unchanged; 211 passed / 2 ignored in workspace, unchanged)

Identical to Phase 25's counts across the board — this slice added no
`.rs` file and modified none, so every crate's test suite is byte-for-byte
what it was last session. `cubism-cli` still has no test harness of its
own; its `cargo build`/`cargo clippy` legs are clean, and this session
additionally exercised it as a real end-user would (`temporal-build` +
`iceberg-build` against a brand-new spec/dataset outside the workspace's
own fixtures), which the standard battery alone does not cover.

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 35 passed, 1 ignored (6 suites) — unchanged
cargo test -p cubism-iceberg --test concurrency                            # 4 passed, 1 ignored — unchanged
cargo test -p cubism-iceberg --test durability                             # 9 passed — unchanged
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings      # clean
cargo build -p cubism-cli                                                  # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings         # clean
cargo test --workspace --exclude cubism-py                                 # 211 passed, 2 ignored (24 suites) — unchanged
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
./examples/web_analytics_demo/build_temporal_demo.sh                       # real run; output pasted above and in README.md
```

Every `cargo` leg's full, non-truncated output was captured to a file
first and grepped for `test result:`/`error` afterward (not piped through
`tail` directly), per the skill's own step-4 note about a truncated
capture producing a wrong total before. Totals summed by hand from each
suite's own `test result:` line match Phase 25's totals exactly.

## Primary files

- [`../examples/web_analytics_demo/web_analytics_temporal.yaml`](../examples/web_analytics_demo/web_analytics_temporal.yaml)
  (new — the `cubism/v2alpha1` spec)
- [`../examples/web_analytics_demo/generate_temporal_events.py`](../examples/web_analytics_demo/generate_temporal_events.py)
  (new — the dataset generator, including the late-arrival split)
- [`../examples/web_analytics_demo/build_temporal_demo.sh`](../examples/web_analytics_demo/build_temporal_demo.sh)
  (new — the reproducible build script)
- [`../examples/web_analytics_demo/README.md`](../examples/web_analytics_demo/README.md)
  ("Time-series variant" section)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 12 marked
  superseded; Milestones 12a/12b added)
- [`TIMESERIES_PHASE_25_HANDOFF.md`](TIMESERIES_PHASE_25_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#20`](https://github.com/jeromebanks/cubism-rs/issues/20)
  (untouched this session, still the top deferred item)
