# Serving a cube: `cubism serve`

The cube parquet written by `cubism run --output` is a complete serving
artifact — one row per cell, presented measures, and mergeable sketch
blobs. `cubism serve` puts a read-only JSON API and a dashboard on top of
one:

```bash
cubism run examples/claude_local.yaml --input claude_events.parquet \
    --output cube.parquet --show 0
cubism serve cube.parquet --port 8080
# -> http://127.0.0.1:8080
```

No spec, no database, no other process: the server derives everything —
dimensions, measures, sketch kinds — from the cube file itself (sketch
kinds come from the magic bytes of the versioned blob formats).

## The dashboard (`/`)

A single self-contained HTML page (embedded in the binary, no external
assets, works offline):

- **Overview** — the `/G` cell's measures as stat cards
- **Slice** — pick dimension × level × measure, get a bar chart; click a
  bar to inspect that cell
- **Set operations** — pick any two cells and a `count_distinct` measure:
  intersection, union, Jaccard, and lift, computed *at request time* from
  the two cells' stored KMV sketches
- **Cell inspector** — any cell's measures plus decoded sketches (top-k
  leaderboard, exemplar chips, distinct-count estimate)
- **Time series** — one line per XUnit across a range of event-time
  buckets, read from a durable Iceberg cube. Hidden unless the server was
  started in temporal mode (see below)

## The API

All endpoints return JSON; errors are `{"error": "message"}` with
400/404 status. Everything in this section is GET; the one POST endpoint
(`/api/series`) is in the temporal-mode section below.

### `GET /api/meta`

```json
{"source": "cube.parquet", "cells": 1227, "has_global": true,
 "dimensions": ["loop", "model", "outcome", "project", "time", "tool"],
 "measures": ["cost", "tokens", "cache_read", "calls", "sessions", "costliest_sessions"],
 "sketches": [{"measure": "sessions", "kind": "kmv"},
              {"measure": "costliest_sessions", "kind": "topk"}]}
```

### `GET /api/cells?dim=tool&depth=1&sort=cost&limit=25`

The single-dimension cells of `dim` with `depth` hierarchy attributes,
sorted by a measure (descending). `[{xunit, label, measures: {...}}]`.

### `GET /api/cell?xunit=/tool/tool_group=builtin/tool=Bash`

One cell: measures plus every sketch decoded to a JSON summary
(`kmv` → estimate + k, `topk` → items, `sample` → values,
`centroid` → dim + mean). Raw blobs are never returned.

### `GET /api/setops?a=<xunit>&b=<xunit>[&measure=sessions]`

The differentiator. Set algebra over two cells that were never aggregated
together, from their stored sketches:

```json
{"measure": "sessions",
 "a": {"xunit": "/tool/tool_group=builtin/tool=Bash", "estimate": 30.0},
 "b": {"xunit": "/outcome/outcome=error", "estimate": 21.0},
 "union": 33.0, "intersection": 18.0, "jaccard": 0.545, "lift": 1.05}
```

`lift` uses the `/G` cell as the population and is `null` for cubes built
without `includeGlobal`.

## Temporal mode: `/api/series` and the time-series panel

Everything above reads one static cube parquet. Given a `cubism/v2alpha1`
spec with a `temporal` section plus the durable Iceberg warehouse and
control store that `cubism iceberg-build` publishes into, `serve` also
mounts `/api/series` and the dashboard's **Time series** panel:

```bash
cubism serve placeholder_cube.parquet --port 8080     --spec web_analytics_temporal.yaml     --warehouse .temporal_build/warehouse     --catalog-db .temporal_build/catalog.sqlite     --control-db .temporal_build/control.sqlite
```

The positional `cube_path` is still required — `CubeStore::from_path`
wants a real parquet with an `xunit` column — even though nothing in
`/api/series` reads it. A one-row placeholder is the usual answer; see
`examples/web_analytics_demo/` for a worked one. Without the four flags,
`serve` behaves exactly as documented above and the Time series panel
stays hidden (the page probes for the route and degrades silently).

### `POST /api/series`

The one non-GET endpoint. Deliberately narrow — see the module doc
comment on `crates/cubism-serve/src/series.rs` for the full rationale:

```json
{"selector": "/geo/country=US", "measure": "avg_revenue",
 "start": 1775433600000000, "end": 1775520000000000,
 "exact": false, "gap_policy": "missing",
 "windows": [{"window_id": "2026-04-06", "bucket_start": 1775433600000000}]}
```

- **One XUnit selector per request.** `SeriesResponse::new` requires
  exactly one and rejects a multi-selector query rather than silently
  merging across lattice cells. Charting several XUnits is a client-side
  fan-out — one request per (selector, day) — not a wider request.
- **Times are unix microseconds**, half-open `[start, end)`, matching
  `EventTime`'s own wire form. There is no RFC3339 parsing here.
- **`windows` is a caller-supplied input, not something the server
  derives** from the time range: no canonical `TimeRange` → `WindowId`
  encoding exists in this codebase, so the caller owns that convention.
  Each entry is assigned to whichever resolution segment contains its
  `bucket_start`. A `window_id` that was never published resolves to
  `missing`, never to an exact point with no data behind it.
- **Measures** must be `avg` or a scalar kind (`count`/`sum`/`min`/`max`).
  Sketch-backed kinds (`count_distinct`, `top_k`, `quantile`) are
  rejected.

The response reports coverage truthfully:

```json
{"cube": "northstar_web_analytics_temporal", "measure": "avg_revenue",
 "source_resolution": "1d",
 "points": [{"bucket_start": 1775433600000000, "bucket_end": 1775520000000000,
             "value": 1.5319148936170213, "is_exact": true, "missing": [],
             "published": [{"window_id": "2026-04-06", "revision": 1}]}]}
```

`is_exact` says whether the point is a complete answer for its bucket,
`missing` names the windows that were not published, and `published`
carries the revision each contributing window was read at — so a
corrected window shows up as revision 2 rather than quietly changing
value.

Note that a request whose range spans several days returns **one** point,
not one per day: contiguous windows merge into a single interior segment.
One request per day is what produces a line.

### The Time series panel

Charts one line per XUnit over a day range. Selectors are separated by
`;` (a comma already joins YPaths inside a single XUnit). Colour is
series identity; dashed lines with hollow markers mean partial coverage
and `✕` marks a bucket with no data, never interpolated across. `log y`
is opt-in, for the common case where a global rollup and a single cell
differ by an order of magnitude. `?selectors=…&measure=…&start=…&end=…`
(and `&log=1`) prefill the controls, so a deployment can hand out a link
that opens on a particular chart.

## Scope and roadmap

Deliberately minimal for now: one cube file (plus, optionally, one
temporal cube), read-only, binds to `127.0.0.1` only (the cube contains whatever your spec put in it — treat
the port as private). This crate is the seed of the SaaS serving plane;
next steps there are multi-cube registries, cube *merging* at load time
(union several members' cubes per `docs/scaling.md`), auth, and
Arrow-native output alongside JSON.
