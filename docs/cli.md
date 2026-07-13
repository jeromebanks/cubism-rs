# CLI

The `cubism` binary (crate `cubism-cli`) drives the engine from the shell.

```bash
cargo build --release -p cubism-cli    # binary at target/release/cubism
```

## `cubism validate <spec.yaml>`

Parse + validate a spec; prints a one-line summary or every validation error
(exit 1). Cheap enough for CI and editor hooks.

```text
$ cubism validate examples/web_events.yaml
OK: cube 'web_events' — 3 dimension(s), 3 measure(s), 2 filter rule(s)

$ cubism validate broken.yaml
invalid cube spec:
filter rule references unknown dimension 'nope' (declared dimensions: geo)
measure 'm': agg 'Sum' requires an 'input' column or expression
```

## `cubism run <spec.yaml> --input <path> [--output <path>] [--show N]`

Build a cube.

| Flag | Meaning |
|---|---|
| `--input` | required; parquet by default, `.csv` by extension (globs work: `data/*.parquet`) |
| `--output` | write the full cube — including `__sketch` blob columns — as parquet |
| `--show N` | print the top N cells by the first measure (default 10, `0` to silence); blob columns are hidden from display but always written to `--output` |

```text
$ cubism run examples/bench_events.yaml --input events.parquet --output cube.parquet --show 3
cube 'bench_events': 13504 cells in 5.7s (227 dictionary entries)
+-------------------------+----------+----------+-------------------+
| xunit                   | pvs      | events   | reach             |
+-------------------------+----------+----------+-------------------+
| /G                      | 99985694 | 10000000 | 955931.0328943529 |
| ...                                                               |
wrote cube.parquet
```

The output parquet is the serving artifact: one row per cell, canonical
`xunit` string, presented measures, and mergeable sketch blobs (see
`docs/sketches.md` for consuming them).

## `cubism serve <cube.parquet> [--port 8080]`

Serve a cube file: a read-only JSON API (`/api/meta`, `/api/cells`,
`/api/cell`, `/api/setops`) plus an embedded dashboard at `/`. Binds to
`127.0.0.1` only. Full endpoint reference: `docs/serving.md`.

```text
$ cubism serve cube.parquet
serving 1227 cells at http://127.0.0.1:8080
```

Exit codes: 0 success, 1 on any error (unreadable input, invalid spec,
execution failure) with the reason on stderr.
