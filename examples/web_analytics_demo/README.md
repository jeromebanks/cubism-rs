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

Verbatim output from `build_temporal_demo.sh`'s own final `sqlite3` step,
captured from a real run (re-run it yourself to confirm — the generator
is seeded, so the numbers below reproduce exactly):

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
(`/api/series`) is Milestone 12b's job — see the next section.

## Query view (`docs/TIMESERIES_ROADMAP.md` Milestone 12b)

`build_temporal_demo.sh` above proves the revision bump landed in the
control store. This script proves it's visible to a real client: it
builds window `2026-04-07` at revision 1 (the initial, late-events-held-
back stream), starts a real `cubism serve --spec .. --warehouse ..`
process, POSTs `/api/series`, then builds revision 2 (the full stream)
**against that same running server** and POSTs the identical request
again.

```bash
# From the cubism/ directory:
./examples/web_analytics_demo/query_temporal_demo.sh
```

This script is self-contained — it does not require
`build_temporal_demo.sh` to have run first, and builds only window
`2026-04-07` (both revisions), not the two ordinary days. Both scripts
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
== /api/series BEFORE the correction (window 2026-04-07 at revision 1) ==
{"cube":"northstar_web_analytics_temporal","measure":"avg_revenue","points":[{"bucket_end":1775606400000000,"bucket_start":1775520000000000,"is_exact":true,"missing":[],"published":[{"revision":1,"window_id":"2026-04-07"}],"value":0.0}],"source_resolution":"1d"}

== /api/series AFTER the correction (window 2026-04-07 at revision 2, same server, no restart) ==
{"cube":"northstar_web_analytics_temporal","measure":"avg_revenue","points":[{"bucket_end":1775606400000000,"bucket_start":1775520000000000,"is_exact":true,"missing":[],"published":[{"revision":2,"window_id":"2026-04-07"}],"value":1.6638655462184875}],"source_resolution":"1d"}
```

`avg_revenue` for `/G` on window `2026-04-07` moves from `0.0` to
`1.6638655462184875` once the 6 held-back `signup_completed` events are
included — every revenue-bearing event that day happened to be among the
6 chosen as "late" by the generator's seed, so revision 1 has zero
revenue rows in view (not a bug, a property of this seed). `0.0` here is
a real average over revenue-free rows, not a masked empty read: Milestone
11's own integration test shows what an empty/unknown-window read
actually looks like over this API — `value: null`, `is_exact: false` —
which is a different shape than what's captured above. Both responses
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
