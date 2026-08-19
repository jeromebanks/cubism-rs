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

1. generates a small (~1,000-event, 3-day) event stream via
   `generate_temporal_events.py` against `web_analytics_temporal.yaml`
   (`apiVersion: cubism/v2alpha1`, one `geo`/`country` dimension, one
   `avg_revenue` `Avg` measure) — deterministic (seeded), written to
   `.temporal_build/` (gitignored, regenerated on every run, same
   "generated output, deliberately not committed" convention as
   `events.csv`);
2. drives `cubism temporal-build` + `cubism iceberg-build` (durable mode:
   `--catalog-db`/`--control-db`) once per day for the two ordinary days,
   and **twice** for the middle day (2026-04-07): revision 1 from an
   "initial" event set with 6 revenue-bearing `signup_completed` events
   held back, revision 2 from the full set once those events are
   included — simulating a late-arriving correction;
3. prints the resulting `control_runs`/`control_publications` rows for
   that window directly from the sqlite control store.

Captured output from a real run (`git log` for the commit this was
captured in):

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
(`/api/series`) is Milestone 12b's job, not this one's — see
`docs/TIMESERIES_ROADMAP.md` for that entry.
