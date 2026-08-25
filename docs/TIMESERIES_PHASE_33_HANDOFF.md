# Time-Series Phase 33 Handoff

Date: 2026-08-25

Branch: `feature/timeseries-phase-0a`

Status: **The demo is now chartable.** Second slice of
`TIMESERIES_FINISH_PROMPT.md`'s demo item (roadmap Milestone 15): the
serve dashboard gained a time-series line-chart panel fed by `/api/series`,
and a capture script proves the whole path — served markup, per-day API
requests, corrected revision visible — end to end. What remains of the
demo item is the docs/wiring slice (sub-part d: README, `docs/serving.md`,
the demo spec gaining a count measure). Zero `.rs` source changes this
slice. (Naming note, carried forward from every prior handoff in this
series: this file's number is a *session-slice* number, not a plan phase
number.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

- **The Time series panel**
  (`crates/cubism-serve/assets/index.html:79`), full-width between Slice
  and the set-ops grid, `hidden` until its first load proves `/api/series`
  is mounted. The probe (`postSeries`, index.html:255) classifies raw
  fetch results: axum's non-JSON unmatched-route 404 means "absent" and
  the panel stays hidden silently — static-cube-only serves degrade
  exactly as before; a JSON `{"error": ...}` envelope shows the error;
  points render. The loader (`loadTimeSeries`, index.html:268) never
  throws, so a series problem cannot reach `init()`'s global catch and
  blank the static panels.
- **One single-day request per day** (`tsRequest`, index.html:241's block)
  — not one multi-day request: `/api/series` merges each contiguous
  segment's windows into ONE point (`range_query.rs`'s "(at most one)
  interior segment"), so a multi-day request renders one dot, not a line.
  Windows follow Milestone 12a's day-string convention with µs bucket
  starts (JS ms ×1000). The measure input defaults to `avg_revenue`
  hardcoded — deliberately never derived from `/api/meta`, whose measures
  belong to the static placeholder cube in temporal mode.
- **Honest SVG encoding** (`drawSeriesChart`, index.html:302), same
  string-building style as the page's existing `barChart()`, no external
  assets: solid accent = exact; dashed amber + hollow markers = partial
  coverage; ✕ on the baseline = missing/no-data (never interpolated
  across); native `<title>` tooltips carry date · PARTIAL · revision ·
  value; legend reports `source_resolution` and revisions shown.
- **`examples/web_analytics_demo/serve_temporal_dashboard.sh`** — builds
  all three days via `build_temporal_demo.sh`, serves behind the
  placeholder-parquet trick (with an interpreter probe for pyarrow, since
  plain `python3` here can be a pyenv shim without it), greps `/` for the
  panel tokens, then issues the exact JSON bodies the JS constructs.
- **Test bump**: `crates/cubism-serve/tests/api.rs:149` asserts the served
  page contains `id="tsPanel"`, `id="tsChart"`, `/api/series`,
  `bucket_start`, `is_exact`, via a new `get_text` helper
  (`crates/cubism-serve/tests/api.rs:155`; the JSON-parsing helper nulls
  out HTML).
- **Roadmap Milestone 15** entry added in the POC section.

Out of scope, unchanged: any `.rs` source (router untouched — no meta
capability flag; client-side detection only), README/docs wiring,
discovery endpoint, multi-selector, demo spec/dataset changes.

## The captured transcript (this slice's evidence)

```text
== dashboard page carries the time-series panel ==
  ok: id="tsPanel"
  ok: /api/series
  ok: bucket_start
  ok: is_exact

-- day 1 (2026-04-06) --
{"cube":"northstar_web_analytics_temporal",...,"points":[{"bucket_end":1775520000000000,
 "bucket_start":1775433600000000,"is_exact":true,"missing":[],
 "published":[{"revision":1,"window_id":"2026-04-06"}],"value":0.6428571428571429}],
 "source_resolution":"1d"}
-- day 2 (2026-04-07) --
... "is_exact":true,... "published":[{"revision":2,"window_id":"2026-04-07"}],
    "value":1.6638655462184875} ...
-- day 3 (2026-04-08) --
... "published":[{"revision":1,"window_id":"2026-04-08"}],"value":1.4918032786885247} ...

all three days answer exactly, with 2026-04-07 at its corrected revision 2
```

Day 2 at **revision 2** with value `1.6638655462184875` is Milestone 12b's
correction, now sitting in the chart's default range next to its
uncorrected neighbors.

## What was actually verified

- Mechanically: the served page carries the panel hooks (cargo test +
  script grep); the API answers the exact per-day request bodies the JS
  issues, three exact points with the middle day at revision 2; the full
  battery below.
- NOT mechanically verified, stated plainly: that a browser executes the
  JS and renders the SVG line chart. That part is human-verified by
  opening the dashboard after running the two scripts (Milestone 12a
  precedent — UI slices are transcript-verified, not cargo-tested).
  Everything the rendering depends on — endpoints, field names, µs
  literals, revision provenance — is pinned by the mechanical layer.

## GitHub issues touched

None filed, none closed. Remaining demo work (docs/wiring slice) is the
finish prompt's own sub-part (d); the discovery endpoint stays deferred
per Milestone 14/15 entries until a real client need beyond the
day-string convention appears.

## Deferred / not done this session

1. **Docs/wiring slice** (finish-prompt sub-part d):
   `examples/web_analytics_demo/README.md` gains the dashboard section;
   `docs/serving.md` documents `serve --spec/--warehouse` and `/api/series`
   (currently zero mention); the demo spec/dataset gain a count measure
   (e.g. `page_views`) so a scalar-kind series exists to chart — the
   panel's free-text measure input makes it chartable with zero UI change.
   Optionally fold in `git add` hygiene for the new script (done this
   session) and a `.temporal_build/` gitignore check.
2. Then per the finish prompt's order: durability tail sweep (#9 checksum,
   #14 retry coverage, guard hoisting, AwaitingAppend repro, SQLite
   refusal leg), #8 re-test spike, #16 engine link, Phase 6, #22 items.

## Worktree state

Committed to `feature/timeseries-phase-0a` (see `git log`; hashes
deliberately not hardcoded here).

- Modified: `crates/cubism-serve/assets/index.html`,
  `crates/cubism-serve/tests/api.rs`, `docs/TIMESERIES_ROADMAP.md`
  (Milestone 15), `docs/TIMESERIES_PHASE_32_HANDOFF.md` (`Superseded by`
  line), `docs/handoff_latest.md` (symlink retarget).
- New: `examples/web_analytics_demo/serve_temporal_dashboard.sh`
  (tracked — a repo asset like its sibling scripts),
  `docs/TIMESERIES_PHASE_33_HANDOFF.md` (this file).
- Untouched: every `.rs` source file under `crates/`.

Deliberately uncommitted, prior-session convention: `.serena/`,
`examples/web_analytics_demo/events.csv`,
`.claude/skills/timeseries-slice/SKILL.md` pre-existing edits,
`docs/CODEX_TELEMETRY_ROADMAP.md`. The capture script's own artifacts
(`.temporal_build/` including its logs) are gitignored.

## Tests (40 passed + 1 ignored in cubism-iceberg; 229 passed / 2 ignored workspace)

Counts unchanged from Phase 32's final state: this slice extended the
existing api test's assertions rather than adding test functions, and
touched no other Rust code.

## Verification performed

```text
cargo test -p cubism-iceberg                    # 40 passed, 1 ignored
cargo test -p cubism-iceberg --test concurrency # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability  # 10 passed, 0 ignored
cargo test --workspace --exclude cubism-py      # 229 passed, 2 ignored
cargo clippy -p cubism-serve --all-targets --no-deps -- -D warnings  # clean
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
cargo build -p cubism-cli                       # clean
./examples/web_analytics_demo/serve_temporal_dashboard.sh  # transcript above
```

## Primary files

- [`../../crates/cubism-serve/assets/index.html`](../crates/cubism-serve/assets/index.html)
  (panel + probe + chart)
- [`../../examples/web_analytics_demo/serve_temporal_dashboard.sh`](../examples/web_analytics_demo/serve_temporal_dashboard.sh)
  (behavioral capture)
- [`../../crates/cubism-serve/tests/api.rs`](../crates/cubism-serve/tests/api.rs)
  (markup-token assertions)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 15)
- [`TIMESERIES_PHASE_32_HANDOFF.md`](TIMESERIES_PHASE_32_HANDOFF.md)
  (prior handoff, superseded by this one)
