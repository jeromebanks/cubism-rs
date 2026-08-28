# Time-Series Phase 34 Handoff

Date: 2026-08-28

Branch: `feature/timeseries-demo-multi-xunit` (branched from
`feature/timeseries-phase-0a` at Phase 33's tip — the first slice in this
series not landed directly on `phase-0a`, at the repo owner's request, so
the remaining work starts from a fresh branch)

Status: **The demo is done and visually verified in a real browser.** The
finish prompt's demo item — all four sub-parts (a) API widening, (b)
discovery, (c) the chart, (d) docs/wiring — is closed. The chart draws
multiple XUnits across 14 days with the corrected day visible at revision
2. (Naming note, carried forward: this file's number is a *session-slice*
number, not a plan phase number.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session found

Phase 33 left the demo one uncommitted edit away from working, and the
edit was already written. In `init()`, `META.dimensions.reduce(...)` ran
*before* `loadTimeSeries()`. The temporal serve runs against a
dimensionless placeholder cube (`/api/meta` returns `"dimensions":[]`), so
`reduce` on an empty array threw, `init()`'s catch swallowed it, and the
time-series panel never unhid — the page rendered with no chart at all.
The worktree's uncommitted `crates/cubism-serve/assets/index.html` fixed
exactly that but was never rebuilt (`DASHBOARD` is `include_str!`-embedded)
or committed, so nobody had seen it work. It is committed here.

That single fault is the whole of "the demo didn't run". Everything else
below is the demo the fault was hiding, made worth looking at.

## What this session built

- **Multi-XUnit charting** (`crates/cubism-serve/assets/index.html`; zero
  `.rs` changes). One line per XUnit, client-side fan-out of one request
  per `(selector, day)` in a single `Promise.all` — 98 requests ≈ 210 ms
  locally. The loop stays client-side deliberately: `SeriesResponse::new`
  requires exactly one selector and
  `series_response_rejects_multi_selector_query` asserts a multi-selector
  query is *rejected, not silently merged*. A server-side `selectors: [..]`
  widening would have to run the same loop and would buy nothing visible.
  The request cap moved from days (92) to selectors × days (200).
- **`;` separates selectors, never `,`.** A comma already joins the YPaths
  *inside* one XUnit — `/geo/country=US,/device/type=mobile` is a single
  two-dimension cell, and one this demo's lattice actually materializes.
  Splitting the input on `,` would have shredded multi-dimension selectors
  into unparseable fragments.
- **Colour re-encoding.** With several series drawn together colour must
  mean "which XUnit", so exactness moved entirely to line style (dashed)
  plus marker fill (hollow). Milestone 15's accent/amber exact-vs-partial
  pair would otherwise read as series identity. The y-domain folds over
  every series so lines stay comparable; `✕` gap marks are per-series
  coloured and offset so several series' gaps stay distinguishable.
- **`log y` toggle.** A global rollup and a single-country cell differ by
  an order of magnitude, flattening the small series onto the axis. Opt-in,
  because it changes what the slope means (equal ratios, not equal
  differences). Values ≤ 0 have no log: they become gaps and the legend
  counts them. The linear path additionally clamps its lower bound at 0
  for all-non-negative data — it was drawing a `-0.28` gridline under a
  revenue average.
- **URL prefill** (`applySeriesParams`):
  `?selectors=…&measure=…&start=…&end=…&log=1`. This is what keeps the
  shipped dashboard generic — it knows nothing about countries or plans;
  the demo's selector list lives in the demo's launch script.
- **A dataset worth charting** (`web_analytics_temporal.yaml`,
  `generate_temporal_events.py`, `build_temporal_demo.sh`): 3 days → 14,
  one dimension → three (`geo`/`device`/`plan` under `max_dimensions: 2`),
  one measure → three (`avg_revenue` `Avg`, `revenue` `Sum`, `page_views`
  `Count` — the kinds `/api/series` answers; no sketch-backed measure,
  which it rejects). ~20,000 events, 49–51 XUnits/day. The build script's
  hardcoded three dates became a loop over a day range.
- **Docs**: `docs/serving.md` gained a temporal-mode section (it had zero
  mention of `--spec`/`--warehouse` or `/api/series`);
  `examples/web_analytics_demo/README.md` gained a dashboard section and
  had its now-stale dataset/verbatim-output blocks corrected; roadmap
  Milestone 16 added with amendment notes on 12a/12b/15.

## Two live traps the dataset change exposed

Both would have produced a script that passes while proving nothing:

1. **`query_temporal_demo.sh` hardcoded window `2026-04-07`** while
   inheriting the generator's `--late-day-offset` default. Moving the
   corrected day to offset 7 would have left it querying an *uncorrected*
   window — its "before" and "after" responses would be identical, and the
   script's entire purpose silently voided.
2. **`serve_temporal_dashboard.sh`** asserted revisions `[1, 2, 1]` on
   hardcoded days `04-06/07/08`. With the corrected day moved it would
   have failed outright (the better of the two failure modes).

Both now derive their days from **`demo_env.sh`**, new and shared by all
four scripts: `START_DATE`/`DAYS`/`USERS`/`LATE_OFFSET`/`day_at()`/
`LATE_DAY`. This file is tracked — every script sources it, so an
explicit-path commit that dropped it would break the demo on a fresh
checkout.

## What was actually verified

- **Mechanically:** 229 workspace tests pass (`--exclude cubism-py`, count
  unchanged from Phase 33 — no test functions added or removed); clippy
  clean on `-D warnings` for the workspace and `cubism-serve`; all four
  demo scripts run end to end from a clean `.temporal_build/`;
  `run_temporal_demo.sh` verified from a foreign cwd (`cd /tmp && …`),
  since `. ./demo_env.sh` relies on the earlier `cd "$(dirname "$0")"`.
- **In a real browser** (Playwright — this closes Phase 33's explicitly
  unverified gap, "NOT mechanically verified: that a browser executes the
  JS and renders the SVG"): rendered SVG with 7 series over 14 days, 98
  `<title>` tooltips, the corrected day `2026-04-13` reading `rev 2` across
  every series while its neighbours read `rev 1`, legend reporting
  `resolution 1d · revisions shown: 1, 2`, and **zero console errors**.
- **Not verified:** anything about concurrent requests racing a publish
  (the read-after-plan non-atomicity `series.rs`'s module doc already
  records), resolutions other than `1d`, or browsers other than the
  Playwright-driven Chromium.

## A claim corrected twice — do not restore it

An earlier draft of the README said the 51 XUnits/day demonstrated
*sparsity*. It does not: 51 is also the full lattice size under
`max_dimensions: 2` (1 global + 11 single-dimension + 39 two-dimension),
and the build materializes 49–51 per day, 12 of 15 windows hitting all 51.
This dataset is nearly dense. What it demonstrates is **rule-constrained
materialization** — the 45 three-dimension cells never exist at all. The
sparse path needs a thinner stream or higher-cardinality dimensions to
show. Recorded here so the demo is not cited as sparsity evidence it does
not provide.

## GitHub issues touched

None filed, none closed. The deferred discovery endpoint (a temporal
XUnit-listing route reading the registry table) stays deferred: URL
prefill gave the demo the affordance without new public API in
`cubism-iceberg`, and `CanonicalXUnit` → `XUnit` reconstruction has no
other caller yet.

## Deferred / not done this session

Unchanged from Phase 33's list, minus the demo item:

1. Durability tail sweep (#9 checksum, #14 retry coverage, guard hoisting,
   AwaitingAppend repro, SQLite refusal leg).
2. #8 re-test spike (DataFusion 55.0.0 / `iceberg-datafusion` pin).
3. **#16 — the engine link.** Still the last big functional gap: nothing
   computes a correction end-to-end. This demo simulates a correction by
   rebuilding from a fuller event set with a hand-supplied `--revision 2`.
4. Phase 6 (rolling comparisons / trend inputs).
5. #22 items (bench timing fix first).

## Worktree state

Committed to `feature/timeseries-demo-multi-xunit`.

- Modified: `crates/cubism-serve/assets/index.html`, `docs/serving.md`,
  `docs/TIMESERIES_ROADMAP.md`, `examples/web_analytics_demo/README.md`,
  `build_temporal_demo.sh`, `generate_temporal_events.py`,
  `query_temporal_demo.sh`, `run_temporal_demo.sh`,
  `serve_temporal_dashboard.sh`, `web_analytics_temporal.yaml`.
- New: `examples/web_analytics_demo/demo_env.sh` (tracked — every script
  sources it), `docs/TIMESERIES_PHASE_34_HANDOFF.md` (this file).
- Untouched: every `.rs` source file under `crates/`.

Deliberately uncommitted, prior-session convention: `.serena/`,
`examples/web_analytics_demo/events.csv`,
`.claude/skills/timeseries-slice/SKILL.md` pre-existing edits,
`docs/CODEX_TELEMETRY_ROADMAP.md`. `.temporal_build/` is gitignored.

## Verification performed

```text
cargo test --workspace --exclude cubism-py                            # 229 passed
cargo test -p cubism-iceberg                                          # 40 passed, 1 ignored
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
./build_temporal_demo.sh          # 14 days, corrected day 2026-04-13 at revision 2
./query_temporal_demo.sh          # avg_revenue 0.39083750894774516 -> 0.9194583036350678
./serve_temporal_dashboard.sh     # revisions 1/2/1 across the correction boundary
./run_temporal_demo.sh --port N   # from the demo dir and from /tmp
```

## Primary files

- [`../../crates/cubism-serve/assets/index.html`](../crates/cubism-serve/assets/index.html)
  (multi-series chart, log axis, URL prefill)
- [`../../examples/web_analytics_demo/demo_env.sh`](../examples/web_analytics_demo/demo_env.sh)
  (the shared shape all four scripts read)
- [`../../examples/web_analytics_demo/README.md`](../examples/web_analytics_demo/README.md)
  (dashboard section)
- [`serving.md`](serving.md) (temporal mode, `/api/series`)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 16)
- [`TIMESERIES_PHASE_33_HANDOFF.md`](TIMESERIES_PHASE_33_HANDOFF.md)
  (prior handoff, superseded by this one)
