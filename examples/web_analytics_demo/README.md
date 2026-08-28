# Cubism web analytics demo

This is a small Google Analytics / Quantcast-style demo for Cubism.

It models a fictional product site, **Northstar**, with acquisition,
behavior, device, geography, and conversion events. Cubism turns those events
into a multidimensional cube and serves an interactive dashboard.

## Run it

From the `cubism/` directory:

```bash
# 1. Generate deterministic web events.
python3 examples/web_analytics_demo/generate_events.py \
  --output examples/web_analytics_demo/events.csv

# 2. Validate and build the Cubism cube.
cargo run --release -p cubism-cli -- validate \
  examples/web_analytics_demo/web_analytics.yaml
cargo run --release -p cubism-cli -- run \
  examples/web_analytics_demo/web_analytics.yaml \
  --input examples/web_analytics_demo/events.csv \
  --output examples/web_analytics_demo/web_analytics_cube.parquet \
  --show 10

# 3. Serve the cube and open http://127.0.0.1:8090.
cargo run --release -p cubism-cli -- serve \
  examples/web_analytics_demo/web_analytics_cube.parquet --port 8090
```

Open `site/index.html` separately to see the small product site that the
events represent. Its event names and columns match the generated data.

## What to demo

The dashboard gives you:

- total events, users, sessions, and revenue from the `/G` rollup;
- slices by acquisition source, device, country, page, or event;
- cell inspection with distinct-count sketches and top pages;
- ad-hoc audience overlap, such as `pricing` visitors ∩ `signup_completed`
  sessions, even though that pair was never materialized as a cube cell.

The last item is the Cubism-specific story: the dashboard computes the
intersection from stored KMV sketches at query time.

## Time-series variant (`docs/TIMESERIES_ROADMAP.md` Milestone 12a)

The static demo above builds one cube from `web_analytics.yaml`
(`apiVersion: v1`) via the non-durable `cubism-cli run` path. That spec
cannot carry a `temporal` section (`v1` rejects one outright), so the
time-series variant is a separate spec, dataset, and build path —
`web_analytics.yaml`/`events.csv`/`web_analytics_cube.parquet` above are
untouched by any of this.

```bash
# From the cubism/ directory:
./examples/web_analytics_demo/build_temporal_demo.sh
```

This single script:

1. generates a ~20,000-event, 14-day stream via
   `generate_temporal_events.py` against `web_analytics_temporal.yaml`
   (`apiVersion: cubism/v2alpha1`; `geo`/`device`/`plan` dimensions under
   a `max_dimensions: 2` rule, and three measures — `avg_revenue` (`Avg`),
   `revenue` (`Sum`), `page_views` (`Count`)) — deterministic (seeded),
   written to `.temporal_build/` (gitignored, regenerated on every run,
   same "generated output, deliberately not committed" convention as
   `events.csv`). The full lattice under that rule is 51 cells — the
   global rollup, 11 single-dimension cells, 39 two-dimension cells — and
   the build materializes 49 to 51 of them per day (12 of the 15 published
   windows hit all 51). So at ~1,400 events/day over 5 countries × 3
   device types × 3 plans this dataset is *nearly* dense: it is not a
   showcase of sparsity, which only bites on thinner streams or
   higher-cardinality dimensions. What it does demonstrate is
   rule-constrained materialization — without `max_dimensions: 2` the
   45 three-dimension cells would exist too, and never do;
2. drives `cubism temporal-build` + `cubism iceberg-build` (durable mode:
   `--catalog-db`/`--control-db`) once per day for the thirteen ordinary
   days, and **twice** for the corrected day (2026-04-13): revision 1 from
   an "initial" event set with 6 revenue-bearing `signup_completed` events
   held back, revision 2 from the full set once those events are
   included — simulating a late-arriving correction;
3. prints the resulting `control_runs`/`control_publications` rows for
   that window directly from the sqlite control store.

The day range, dataset size and corrected day all come from
`demo_env.sh`, shared by the three scripts here, so
`DAYS=30 ./build_temporal_demo.sh` reshapes the whole demo consistently.

Verbatim output from `build_temporal_demo.sh`'s own final `sqlite3` step,
captured from a real run (re-run it yourself to confirm — the generator
is seeded, so the numbers below reproduce exactly):

```text
== control_runs rows for window 2026-04-13 (revision bump) ==
run_id             window_id   revision  status
-----------------  ----------  --------  ---------
run-2026-04-13-r1  2026-04-13  1         published
run-2026-04-13-r2  2026-04-13  2         published

== control_publications current pointer for window 2026-04-13 ==
cube_id                           window_id   revision  run_id
--------------------------------  ----------  --------  -----------------
northstar_web_analytics_temporal  2026-04-13  2         run-2026-04-13-r2

built 14 day(s): 2026-04-06 .. 2026-04-19  (corrected day: 2026-04-13, revision 2)
```

Two published revisions of the same `window_id`, and the control store's
current pointer visibly moved from revision 1 to revision 2 once the
late-arriving events were included in the rebuild.

**What this does and does not prove:** this demonstrates a revision bump
driven by re-running `temporal-build`/`iceberg-build` over a fuller event
set — a real, durable correction. It does **not** exercise
`LatenessPolicy` or `CorrectionPlan`: `iceberg-build` never consults
`temporal.allowedLateness`, and `CorrectionCoordinator` is not in this
script's call path at all. The "late" event is simply absent from
`events_temporal_initial.csv` and present in `events_temporal.csv` — the
demo simulates *what a late correction looks like once it has already
arrived and been rebuilt*, not the policy machinery that would decide
whether to admit it. Querying the corrected value back out over HTTP
(`/api/series`) is Milestone 12b's job — see the next section.

## Query view (`docs/TIMESERIES_ROADMAP.md` Milestone 12b)

`build_temporal_demo.sh` above proves the revision bump landed in the
control store. This script proves it's visible to a real client: it
builds the corrected window (`2026-04-13` with the shared defaults) at
revision 1 (the initial, late-events-held-back stream), starts a real
`cubism serve --spec .. --warehouse ..`
process, POSTs `/api/series`, then builds revision 2 (the full stream)
**against that same running server** and POSTs the identical request
again.

```bash
# From the cubism/ directory:
./examples/web_analytics_demo/query_temporal_demo.sh
```

This script is self-contained — it does not require
`build_temporal_demo.sh` to have run first, and builds only the corrected
window (both revisions), not the other thirteen days. It takes the day it
queries from the same `demo_env.sh` the generator's `--late-day-offset`
comes from — hardcoding a date here is exactly how this script would stop
proving anything, since a "before" and "after" on an uncorrected window
are two identical responses. Both scripts
share the `.temporal_build/` output directory; each does its own `rm -rf`
at the top, so the last one run "wins" as that directory's current state
(same ephemeral-output convention Milestone 12a established).

`cubism serve` requires a positional `cube_path` argument even when only
`--spec`/`--warehouse` (the `/api/series` routes) are wanted —
`CubeStore::from_path` still requires a real parquet with an `xunit`
column, though nothing in `/api/series`'s own handler reads it. Rather
than depend on the *static* demo's `web_analytics_cube.parquet`
(untracked — only present if that demo has been built locally, which
would silently fail from a clean checkout), this script writes its own
throwaway one-row placeholder parquet into `.temporal_build/`.

**Requires `pyarrow`** (`pip install pyarrow`) to write that placeholder
parquet — the only non-stdlib Python dependency anywhere in this demo
(`generate_events.py` and `generate_temporal_events.py` are both stdlib-
only). Also requires `python3`, `curl`, and `sqlite3` on `PATH`, same as
`build_temporal_demo.sh`.

Verbatim output from a real run (re-run it yourself to confirm — the
generator is seeded, so the request/response pair reproduces exactly;
`snapshot`/timing values in the build steps above it will differ run to
run, but are not part of what this section proves):

```text
== /api/series BEFORE the correction (window 2026-04-13 at revision 1) ==
{"cube":"northstar_web_analytics_temporal","measure":"avg_revenue","points":[{"bucket_end":1775606400000000,"bucket_start":1775520000000000,"is_exact":true,"missing":[],"published":[{"revision":1,"window_id":"2026-04-13"}],"value":0.39083750894774516}],"source_resolution":"1d"}

== /api/series AFTER the correction (window 2026-04-13 at revision 2, same server, no restart) ==
{"cube":"northstar_web_analytics_temporal","measure":"avg_revenue","points":[{"bucket_end":1775606400000000,"bucket_start":1775520000000000,"is_exact":true,"missing":[],"published":[{"revision":2,"window_id":"2026-04-13"}],"value":0.9194583036350678}],"source_resolution":"1d"}
```

`avg_revenue` for `/G` on window `2026-04-13` moves from
`0.39083750894774516` to `0.9194583036350678` once the 6 held-back
`signup_completed` events are included — the corrected day's average
revenue per event more than doubles. Neither value is a masked empty
read: Milestone 11's own integration test shows what an empty/unknown-
window read actually looks like over this API — `value: null`,
`is_exact: false` — which is a different shape than what's captured
above. Both responses
report `is_exact: true` and the correct real revision in `published`,
same server process throughout — the second query was answered without a
restart: this session confirmed empirically, before writing the script,
that the next request against an already-running `cubism serve` process
returns the newly published revision and the corrected value, with no
process restart in between.

**What this does and does not prove:** a real HTTP client querying
`avg_revenue` before and after Milestone 12a's revision bump sees the
value change, both times `is_exact: true` with the correct revision
reported. It does not prove anything about concurrent requests racing a
publish (the read-after-plan non-atomicity `crate::series`'s own module
doc comment already documents), about resolutions other than `1d`, or
about any selector other than the global `/G` rollup — narrower than a
real client would use, matching Milestone 11's own narrowing.

## The dashboard (`docs/TIMESERIES_ROADMAP.md` Milestone 15)

The two scripts above prove the data is correct and queryable. This one
makes it *visible*: a time-series line chart of aggregates across the
published days, several XUnits at once.

```bash
# From the cubism/ directory:
./examples/web_analytics_demo/run_temporal_demo.sh --open
```

One command: it builds the days if they don't exist yet (reusing them if
they do — pass `--rebuild` to force), writes the placeholder parquet,
starts `cubism serve`, and prints a URL that already carries the demo's
XUnit list, measure and day range:

```text
  dashboard: http://127.0.0.1:8080/?selectors=/G%3B/geo/country=US%3B...&measure=avg_revenue&start=2026-04-06&end=2026-04-19
  days:      2026-04-06 .. 2026-04-19
  measure:   avg_revenue   (also try: revenue, page_views)
  xunits:    /G;/geo/country=US;/geo/country=GB;/geo/country=DE;/device/type=mobile;/device/type=desktop;/plan/plan=pro
  corrected: 2026-04-13 (serving revision 2)
```

The **Time series** panel charts one line per XUnit over the day range,
each fed by `/api/series`. Things worth noticing in it:

- **Selectors are separated by `;`, never `,`** — a comma already joins
  the YPaths *inside* one XUnit, so `/geo/country=US,/device/type=mobile`
  is a single two-dimension cell (a valid selector here, and one this
  demo's `max_dimensions: 2` lattice actually materializes), not two.
- **Colour means "which XUnit"; exactness is carried by line style** —
  dashed with a hollow marker is partial coverage, `✕` on the baseline is
  a bucket with no data, never interpolated across. The legend reports
  the source resolution and which revisions are on screen.
- **`log y`** — a global rollup and a single-country cell differ by an
  order of magnitude, which flattens the small series onto the axis under
  a shared linear scale. Opt-in, because it changes what the slope means.
- **The corrected day** (`2026-04-13`) sits in the default range next to
  its uncorrected neighbours, serving revision 2; hover any point for its
  date, revision and value, and the legend's "revisions shown: 1, 2"
  reflects that mix.

The dashboard ships generic — it knows nothing about countries or plans.
The demo-specific selector list reaches it through `?selectors=...`
(`applySeriesParams` in `crates/cubism-serve/assets/index.html`), so the
same page serves any temporal cube.

**Pass `--rebuild` if you last ran `query_temporal_demo.sh`.** That
script builds only the corrected window into the same `.temporal_build/`,
so the reuse check here is satisfied by a one-window store and the chart
collapses to a single point. `--rebuild` restores the full day range.

**Requires `pyarrow`**, same as `query_temporal_demo.sh` and for the same
reason (the placeholder parquet). The static panels above the chart
(Slice, Set operations) stay empty in temporal mode: they read the
placeholder cube, which has no dimensions or sketches. That is expected,
and each says so in place rather than failing the page.
