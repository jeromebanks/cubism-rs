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

## The API

All endpoints are GET and return JSON; errors are
`{"error": "message"}` with 400/404 status.

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

## Scope and roadmap

Deliberately minimal for now: one cube file, read-only, binds to
`127.0.0.1` only (the cube contains whatever your spec put in it — treat
the port as private). This crate is the seed of the SaaS serving plane;
next steps there are multi-cube registries, cube *merging* at load time
(union several members' cubes per `docs/scaling.md`), auth, and
Arrow-native output alongside JSON.
